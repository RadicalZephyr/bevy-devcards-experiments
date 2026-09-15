// wgpu's type graph is deep enough that auto-trait resolution for
// `RenderDevice: Resource` blows the default limit.
#![recursion_limit = "256"]

//! E31 — cross-App texture handoff.
//!
//! Gate 2, and after Gate 1 failed this is the experiment that decides the
//! product's shape. The plan frames it as: "Card app renders to an `Image`. Both
//! apps share the device so the underlying texture is usable by both, but
//! `RenderAssets` is per-app. Can the host's render world be handed a `GpuImage`
//! pointing at the card's texture without reaching past what Bevy exposes?"
//!
//! The handoff is done the *other way round*, which turns out to be much
//! cleaner. Rather than pushing the card's texture into the host's
//! `RenderAssets`, the host owns the texture and the card renders into it:
//!
//!   1. the host creates an ordinary `Image` and lets its own render world
//!      build the `GpuImage` for it,
//!   2. that `GpuImage`'s `TextureView` is handed to the card app's
//!      `ManualTextureViews`,
//!   3. the card's camera targets `RenderTarget::TextureView(handle)`.
//!
//! The host then samples its own `Image` like any other texture — no injection
//! into the host's asset storage at all, and nothing has to fight the host's
//! render-asset pipeline for ownership of an id.
//!
//! `RenderTarget::TextureView` is documented as the hook for views "created
//! outside of Bevy, for example OpenXR", so this is a supported path rather than
//! a crack we are squeezing through.
//!
//! Verified by GPU readback, not by eye: the card clears to magenta, and the
//! host reads its own texture back and checks the pixels.

use std::sync::{Arc, Mutex};

use bevy::asset::RenderAssetUsages;
use bevy::camera::{ManualTextureViewHandle, RenderTarget};
use bevy::prelude::*;
use bevy::render::gpu_readback::{Readback, ReadbackComplete};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::{
    Extent3d, PollType, TextureDimension, TextureFormat, TextureUsages,
};
use bevy::render::renderer::{
    RenderAdapter, RenderAdapterInfo, RenderDevice, RenderInstance, RenderQueue,
};
use bevy::render::settings::RenderCreation;
use bevy::render::texture::{GpuImage, ManualTextureView, ManualTextureViews};
use bevy::render::{RenderApp, RenderPlugin};
use bevy::window::ExitCondition;

use bevy_devcards_experiments::{BEVY_VERSION, PROFILE};

const SIZE: u32 = 64;
const VIEW_HANDLE: ManualTextureViewHandle = ManualTextureViewHandle(31);
/// What the host fills each texture with before any card touches it.
const INITIAL: [u8; 4] = [0, 0, 0, 255];
const SETTLE_FRAMES: usize = 8;
const CARDS: usize = 3;

fn main() {
    println!("E31 — cross-App texture handoff");
    println!("bevy {BEVY_VERSION}  profile={PROFILE}");
    println!();

    // --- host --------------------------------------------------------------
    let mut host = build_app(None);
    let Some(borrowed) = borrow_render_resources(&host) else {
        println!("no render device — E31 needs a GPU. Stopping.");
        return;
    };
    println!(
        "adapter: {}",
        host.world()
            .resource::<RenderAdapterInfo>()
            .name
            .to_string()
    );
    println!();

    // --- the host owns N textures, one per card -----------------------------
    // Pure channels, so the linear/sRGB round trip through the clear is exact
    // and a mismatch is a real failure rather than a rounding artefact. Distinct
    // colours also make crosstalk between cards visible: if card 1's output
    // landed in card 2's texture, the check below would catch it.
    let colours: [[u8; 4]; CARDS] = [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255]];

    let targets: Vec<Handle<Image>> = (0..CARDS).map(|_| host_texture(&mut host)).collect();

    // One host frame so the render world builds a `GpuImage` for each.
    host.update();
    wait(&host);

    println!("stage 1 — control read, before any card has rendered");
    let mut control_ok = true;
    for (i, target) in targets.iter().enumerate() {
        match readback(&mut host, target) {
            Some(pixels) => {
                let px = centre_pixel(&pixels);
                println!("  texture {i}: {px:?} (expected {INITIAL:?})");
                control_ok &= px == INITIAL;
            }
            None => {
                println!("  FAIL: readback produced nothing — cannot verify anything.");
                return;
            }
        }
    }
    if !control_ok {
        println!("  note: a control read did not match; the comparison below is weaker");
    }
    println!();

    // --- hand each host TextureView to its own App --------------------------
    println!("stage 2 — handing each host TextureView to its own App");
    let mut cards = Vec::new();
    for (i, target) in targets.iter().enumerate() {
        let texture_view = {
            let render_world = host.sub_app(RenderApp).world();
            let gpu_images = render_world.resource::<RenderAssets<GpuImage>>();
            match gpu_images.get(target) {
                Some(gpu_image) => gpu_image.texture_view.clone(),
                None => {
                    println!("  FAIL: the host's render world has no GpuImage for target {i}.");
                    return;
                }
            }
        };

        let mut card = build_app(Some(&borrowed));
        card.world_mut()
            .resource_mut::<ManualTextureViews>()
            .insert(
                VIEW_HANDLE,
                ManualTextureView {
                    texture_view,
                    size: UVec2::splat(SIZE),
                    view_format: TextureFormat::Rgba8UnormSrgb,
                },
            );
        let [r, g, b, _] = colours[i];
        card.world_mut().spawn((
            Camera2d,
            Camera {
                clear_color: ClearColorConfig::Custom(Color::srgb_u8(r, g, b)),
                ..default()
            },
            RenderTarget::TextureView(VIEW_HANDLE),
            Transform::IDENTITY,
        ));
        cards.push(card);
    }
    println!("  {CARDS} card Apps booted on the host's borrowed device (E30's mechanism)");
    println!("  each targets RenderTarget::TextureView into a texture the HOST owns");
    println!();

    // --- run ----------------------------------------------------------------
    println!("stage 3 — cards render, host reads its own textures back");
    for _ in 0..SETTLE_FRAMES {
        for card in &mut cards {
            card.update();
        }
        wait(&host);
    }

    let mut all_ok = true;
    for (i, target) in targets.iter().enumerate() {
        match readback(&mut host, target) {
            Some(pixels) => {
                let px = centre_pixel(&pixels);
                let uniform = pixels.chunks_exact(4).all(|p| p == colours[i]);
                let ok = px == colours[i] && uniform;
                all_ok &= ok;
                println!(
                    "  texture {i}: {px:?} (expected {:?})  uniform={uniform}  {}",
                    colours[i],
                    if ok { "ok" } else { "MISMATCH" }
                );
            }
            None => {
                all_ok = false;
                println!("  texture {i}: FAIL — readback produced nothing");
            }
        }
    }
    println!();

    if all_ok {
        println!("E31 PASS — {CARDS} separate Apps each rendered into the host's own");
        println!("textures, in one process, on one device, with no crosstalk.");
        println!();
        println!("  Everything used is public API:");
        println!("    RenderCreation::manual(..)            borrow the device");
        println!("    RenderAssets<GpuImage>::get(..)       read the host's TextureView");
        println!("    ManualTextureViews (Deref to HashMap) hand it to the card");
        println!("    RenderTarget::TextureView(handle)     point the card's camera at it");
        println!("  No private items, nothing reached past Bevy into wgpu.");
        println!("  This is a crate, not an upstream PR.");
    } else {
        println!("E31 FAIL — the handoff compiled and ran, so the obstacle is");
        println!("behavioural rather than a matter of visibility. Report it as such.");
    }
}

/// An ordinary host-owned `Image`, usable as a render attachment and readable back.
fn host_texture(host: &mut App) -> Handle<Image> {
    let mut image = Image::new_fill(
        Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        &INITIAL,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_descriptor.usage |=
        TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC | TextureUsages::TEXTURE_BINDING;
    host.world_mut().resource_mut::<Assets<Image>>().add(image)
}

// ---------------------------------------------------------------------------

fn build_app(borrowed: Option<&BorrowedRenderResources>) -> App {
    let render_creation = match borrowed {
        Some(res) => RenderCreation::manual(
            res.device.clone(),
            res.queue.clone(),
            res.adapter_info.clone(),
            res.adapter.clone(),
            res.instance.clone(),
        ),
        None => RenderCreation::Automatic(default()),
    };
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: None,
                exit_condition: ExitCondition::DontExit,
                ..default()
            })
            .set(RenderPlugin {
                render_creation,
                synchronous_pipeline_compilation: true,
                ..default()
            }),
    );
    app.finish();
    app.cleanup();
    app
}

struct BorrowedRenderResources {
    device: RenderDevice,
    queue: RenderQueue,
    adapter_info: RenderAdapterInfo,
    adapter: RenderAdapter,
    instance: RenderInstance,
}

fn borrow_render_resources(app: &App) -> Option<BorrowedRenderResources> {
    let world = app.world();
    let render_world = app.get_sub_app(RenderApp)?.world();
    Some(BorrowedRenderResources {
        device: world.get_resource::<RenderDevice>()?.clone(),
        queue: world.get_resource::<RenderQueue>()?.clone(),
        adapter_info: world.get_resource::<RenderAdapterInfo>()?.clone(),
        adapter: world.get_resource::<RenderAdapter>()?.clone(),
        instance: render_world.get_resource::<RenderInstance>()?.clone(),
    })
}

fn wait(app: &App) {
    if let Some(device) = app.world().get_resource::<RenderDevice>() {
        let _ = device.wgpu_device().poll(PollType::Wait {
            submission_index: None,
            timeout: None,
        });
    }
}

/// Read the image back through Bevy's own `Readback`, which is asynchronous —
/// the result arrives on an observer a frame or two later.
fn readback(app: &mut App, handle: &Handle<Image>) -> Option<Vec<u8>> {
    let sink: Arc<Mutex<Option<Vec<u8>>>> = Arc::default();
    let writer = Arc::clone(&sink);
    let entity = app
        .world_mut()
        .spawn(Readback::texture(handle.clone()))
        .observe(move |event: On<ReadbackComplete>| {
            *writer.lock().unwrap() = Some(event.data.clone());
        })
        .id();

    for _ in 0..SETTLE_FRAMES {
        app.update();
        wait(app);
        if sink.lock().unwrap().is_some() {
            break;
        }
    }
    app.world_mut().entity_mut(entity).despawn();
    let result = sink.lock().unwrap().clone();
    result
}

fn centre_pixel(pixels: &[u8]) -> [u8; 4] {
    let offset = ((SIZE as usize / 2) * SIZE as usize + SIZE as usize / 2) * 4;
    pixels
        .get(offset..offset + 4)
        .map(|p| [p[0], p[1], p[2], p[3]])
        .unwrap_or_default()
}
