// wgpu's type graph is deep enough that auto-trait resolution for
// `RenderDevice: Resource` blows the default limit.
#![recursion_limit = "256"]

//! E32 — the duplication tax.
//!
//! "Each App gets its own `AssetServer` and `Assets<T>`. Load the same 20MB mesh
//! in four apps and watch VRAM. If it's 4x, the grid has a hard ceiling that
//! isn't about frame time."
//!
//! After E31 this is the last experiment that could still force a redesign, so
//! it measures two things rather than one:
//!
//!   - **the tax**: four Apps each independently loading the same large texture,
//!   - **the fix**: four Apps sharing one GPU texture, which is possible because
//!     they already share the device.
//!
//! VRAM is read from wgpu's own allocator report rather than inferred from the
//! bytes we asked for, so the numbers include whatever padding and alignment the
//! driver actually applied.
//!
//! A texture rather than the plan's mesh: the duplication question is identical
//! — one `RenderAssets<A>` per App either way — and a texture needs no material
//! or pipeline to get uploaded, which keeps the measurement clean.

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::{
    Extent3d, PollType, TextureDimension, TextureFormat, TextureUsages,
};
use bevy::render::renderer::{
    RenderAdapter, RenderAdapterInfo, RenderDevice, RenderInstance, RenderQueue,
};
use bevy::render::settings::RenderCreation;
use bevy::render::texture::GpuImage;
use bevy::render::{RenderApp, RenderPlugin};
use bevy::window::ExitCondition;

use bevy_devcards_experiments::{BEVY_VERSION, PROFILE};

/// 2048x2048 RGBA8 = 16 MiB, the same order as the plan's 20MB mesh and a clean
/// power of two so driver padding does not muddy the comparison.
const SIZE: u32 = 2048;
const BYTES: u64 = (SIZE as u64) * (SIZE as u64) * 4;
const CARDS: usize = 4;
const SETTLE_FRAMES: usize = 4;

fn main() {
    println!("E32 — the duplication tax");
    println!("bevy {BEVY_VERSION}  profile={PROFILE}");
    println!(
        "asset: {SIZE}x{SIZE} RGBA8 = {} MiB per copy",
        BYTES / (1 << 20)
    );
    println!();

    let mut host = build_app(None);
    let Some(borrowed) = borrow_render_resources(&host) else {
        println!("no render device — E32 needs a GPU. Stopping.");
        return;
    };
    println!(
        "adapter: {}",
        host.world().resource::<RenderAdapterInfo>().name
    );

    host.update();
    wait(&host);

    let Some(baseline) = vram(&borrowed.device) else {
        println!();
        println!("wgpu's allocator report is unavailable on this backend, so VRAM");
        println!("cannot be measured here. Rerun on a Vulkan backend.");
        return;
    };
    println!("baseline VRAM: {}", mib(baseline));
    println!();

    // --- the tax ------------------------------------------------------------
    println!("stage 1 — {CARDS} Apps each loading their own copy");
    let mut taxed = Vec::new();
    for i in 0..CARDS {
        let mut card = build_app(Some(&borrowed));
        let _handle = add_large_texture(&mut card);
        settle(&mut card);
        let now = vram(&borrowed.device).unwrap_or(0);
        println!(
            "  card {i}: VRAM {} (+{} since previous)",
            mib(now),
            mib(now.saturating_sub(taxed.last().copied().unwrap_or(baseline)))
        );
        taxed.push(now);
        // Keep the App alive, otherwise its textures are freed and the next
        // measurement is meaningless.
        std::mem::forget(card);
    }
    let tax_total = taxed.last().copied().unwrap_or(baseline) - baseline;
    println!("  total for {CARDS} copies: {}", mib(tax_total));
    println!(
        "  that is {:.2}x the {} a single copy costs",
        tax_total as f64 / BYTES as f64,
        mib(BYTES)
    );
    println!();

    // --- the fix ------------------------------------------------------------
    // The Apps already share a device, so they can share the texture behind it.
    // `RenderAssets<GpuImage>` is public and `GpuImage` is cheap to clone — the
    // clone shares the same underlying `wgpu::Texture`.
    println!("stage 2 — {CARDS} Apps sharing one GPU texture");
    let before_shared = vram(&borrowed.device).unwrap_or(0);

    let mut owner = build_app(Some(&borrowed));
    let owner_handle = add_large_texture(&mut owner);
    settle(&mut owner);
    let after_owner = vram(&borrowed.device).unwrap_or(0);
    println!(
        "  owner App uploaded the texture: +{}",
        mib(after_owner.saturating_sub(before_shared))
    );

    let shared = owner
        .sub_app(RenderApp)
        .world()
        .resource::<RenderAssets<GpuImage>>()
        .get(&owner_handle)
        .cloned();
    let Some(shared) = shared else {
        println!("  FAIL: the owner App has no GpuImage for its texture.");
        return;
    };

    let mut borrowers = Vec::new();
    for i in 0..CARDS {
        let mut card = build_app(Some(&borrowed));
        // A 1x1 placeholder just to mint an `AssetId` this App owns; its own
        // `GpuImage` is then replaced by the shared one. `prepare_assets` only
        // revisits assets that changed, so the replacement sticks.
        let placeholder = add_placeholder(&mut card);
        settle(&mut card);
        card.sub_app_mut(RenderApp)
            .world_mut()
            .resource_mut::<RenderAssets<GpuImage>>()
            .insert(&placeholder, shared.clone());
        settle(&mut card);
        let now = vram(&borrowed.device).unwrap_or(0);
        println!(
            "  card {i} took the shared texture: +{} since the owner uploaded it",
            mib(now.saturating_sub(after_owner))
        );
        borrowers.push(now);
    }
    let shared_total = borrowers.last().copied().unwrap_or(after_owner) - before_shared;
    println!(
        "  total for 1 owner + {CARDS} borrowers: {}",
        mib(shared_total)
    );
    println!(
        "  that is {:.2}x a single copy",
        shared_total as f64 / BYTES as f64
    );
    println!();

    // --- verdict ------------------------------------------------------------
    let tax_ratio = tax_total as f64 / BYTES as f64;
    let shared_ratio = shared_total as f64 / BYTES as f64;
    println!("verdict");
    println!("  independent loading: {tax_ratio:.2}x   sharing: {shared_ratio:.2}x");
    if tax_ratio > 2.0 && shared_ratio < 2.0 {
        println!();
        println!("  The tax is real and it is avoidable. Loading independently costs");
        println!("  one full copy per App, but the Apps share a device, so one upload");
        println!("  can back all of them through the public `RenderAssets` API.");
        println!();
        println!("  So the grid's ceiling is NOT set by asset duplication — provided");
        println!("  the crate owns asset loading rather than letting each card call");
        println!("  its own AssetServer. That is an API requirement, not an");
        println!("  optimisation: it has to be designed in from the start.");
    } else if tax_ratio <= 2.0 {
        println!();
        println!("  No meaningful tax: independent loads did not scale with App count.");
    } else {
        println!();
        println!("  The tax is real and sharing did NOT avoid it. The grid has a hard");
        println!("  ceiling that is about memory rather than frame time.");
    }

    std::mem::forget(owner);
    for card in borrowers {
        let _ = card;
    }
}

// ---------------------------------------------------------------------------

fn add_large_texture(app: &mut App) -> Handle<Image> {
    let mut image = Image::new_fill(
        Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        &[128, 64, 32, 255],
        TextureFormat::Rgba8UnormSrgb,
        // RENDER_WORLD only: the CPU-side copy is dropped after upload, so what
        // is measured is VRAM rather than RAM.
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_descriptor.usage |= TextureUsages::TEXTURE_BINDING;
    app.world_mut().resource_mut::<Assets<Image>>().add(image)
}

fn add_placeholder(app: &mut App) -> Handle<Image> {
    let image = Image::new_fill(
        Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        &[0, 0, 0, 255],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    app.world_mut().resource_mut::<Assets<Image>>().add(image)
}

fn settle(app: &mut App) {
    for _ in 0..SETTLE_FRAMES {
        app.update();
    }
    wait(app);
}

/// Bytes currently allocated according to wgpu's own allocator.
fn vram(device: &RenderDevice) -> Option<u64> {
    device
        .wgpu_device()
        .generate_allocator_report()
        .map(|report| report.total_allocated_bytes)
}

fn mib(bytes: u64) -> String {
    format!("{:.1} MiB", bytes as f64 / (1u64 << 20) as f64)
}

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
