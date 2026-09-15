// wgpu's type graph is deep enough that auto-trait resolution for
// `RenderDevice: Resource` blows the default limit.
#![recursion_limit = "256"]

//! E01 — render-to-texture camera cost.
//!
//! "N cameras in one world, each targeting a distinct `Image`, at 128x128 and at
//! 512x512. Where does the frame budget go?"
//!
//! This bounds the grid *regardless of which approach wins*, which is why it is
//! worth knowing before investing in Phase 3. Together with E00 it decides Gate
//! 3: if far more worlds can tick than thumbnails can draw, the design is "many
//! simulating, few visible" and the UI has to be built around that asymmetry.
//!
//! One camera per target, all in a single world, sharing one small scene. That
//! isolates the per-camera cost — a separate render pass, view uniform, and
//! target texture each — from scene complexity, which is what the plan asks
//! for. It is a floor: real cards draw more.

use std::time::Instant;

use bevy::asset::RenderAssetUsages;
use bevy::camera::RenderTarget;
use bevy::prelude::*;
use bevy::render::RenderPlugin;
use bevy::render::render_resource::{
    Extent3d, PollType, TextureDimension, TextureFormat, TextureUsages,
};
use bevy::render::renderer::RenderDevice;
use bevy::window::ExitCondition;

use bevy_devcards_experiments::{BEVY_VERSION, PROFILE};

const FRAME_BUDGET_MS: f64 = 1000.0 / 60.0;
const WARMUP_FRAMES: usize = 20;
const FRAMES: usize = 60;
const SPRITES: usize = 24;
const COUNTS: [usize; 7] = [1, 2, 4, 8, 16, 32, 64];
const RESOLUTIONS: [u32; 2] = [128, 512];

fn main() {
    println!("E01 — render-to-texture camera cost");
    println!("bevy {BEVY_VERSION}  profile={PROFILE}");
    if PROFILE == "debug" {
        println!("  WARNING: debug build. Rerun with --release before quoting.");
    }

    let probe = build_app();
    let adapter_name = probe
        .world()
        .get_resource::<bevy::render::renderer::RenderAdapterInfo>()
        .map(|info| format!("{:?} {}", info.device_type, info.name))
        .unwrap_or_else(|| "<no adapter>".into());
    println!("  adapter: {adapter_name}");
    drop(probe);
    println!();

    for size in RESOLUTIONS {
        println!("{size}x{size}:");
        println!(
            "  {:>8} {:>12} {:>12} {:>10}",
            "cameras", "ms/frame", "ms/camera", "in budget"
        );
        let mut ceiling = None;
        for count in COUNTS {
            let ms = measure(count, size);
            let per_camera = ms / count as f64;
            let fits = ms <= FRAME_BUDGET_MS;
            if !fits && ceiling.is_none() {
                ceiling = Some(count);
            }
            println!(
                "  {count:>8} {ms:>12.3} {per_camera:>12.3} {:>10}",
                if fits { "yes" } else { "NO" }
            );
        }
        match ceiling {
            Some(first_over) => println!(
                "  -> budget exceeded between {} and {first_over} cameras",
                first_over / 2
            ),
            None => println!(
                "  -> {} cameras still fit in budget",
                COUNTS[COUNTS.len() - 1]
            ),
        }
        println!();
    }

    println!("  Gate 3: fewer than six simultaneous thumbnails in budget means the");
    println!("  grid isn't a grid. Compare against E00's world-tick ceiling.");
}

/// Milliseconds per frame with `count` cameras each rendering to its own
/// `size`x`size` image.
fn measure(count: usize, size: u32) -> f64 {
    let mut app = build_app();
    scene(&mut app, count, size);

    for _ in 0..WARMUP_FRAMES {
        app.update();
    }
    wait_for_gpu(&app);

    let start = Instant::now();
    for _ in 0..FRAMES {
        app.update();
    }
    // The frame is not finished when `update` returns; the submission is still
    // in flight. Without this the numbers measure CPU-side queueing only.
    wait_for_gpu(&app);
    start.elapsed().as_secs_f64() * 1000.0 / FRAMES as f64
}

fn wait_for_gpu(app: &App) {
    if let Some(device) = app.world().get_resource::<RenderDevice>() {
        let _ = device.wgpu_device().poll(PollType::Wait {
            submission_index: None,
            timeout: None,
        });
    }
}

fn build_app() -> App {
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: None,
                exit_condition: ExitCondition::DontExit,
                ..default()
            })
            .set(RenderPlugin {
                synchronous_pipeline_compilation: true,
                ..default()
            }),
    );
    app.finish();
    app.cleanup();
    app
}

fn scene(app: &mut App, cameras: usize, size: u32) {
    // One shared 4x4 texture for every sprite, so sprite batching and texture
    // upload are constant across the sweep and only the camera count varies.
    let sprite_image = {
        let mut images = app.world_mut().resource_mut::<Assets<Image>>();
        images.add(Image::new_fill(
            extent(4, 4),
            TextureDimension::D2,
            &[255, 255, 255, 255],
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::RENDER_WORLD,
        ))
    };

    for i in 0..SPRITES {
        let f = i as f32;
        app.world_mut().spawn((
            Sprite {
                image: sprite_image.clone(),
                custom_size: Some(Vec2::splat(8.0)),
                ..default()
            },
            Transform::from_xyz(
                (f % 6.0) * 10.0 - 30.0,
                (f / 6.0).floor() * 10.0 - 20.0,
                0.0,
            ),
        ));
    }

    for _ in 0..cameras {
        let target = {
            let mut image = Image::new_uninit(
                extent(size, size),
                TextureDimension::D2,
                TextureFormat::Rgba8UnormSrgb,
                RenderAssetUsages::RENDER_WORLD,
            );
            image.texture_descriptor.usage |= TextureUsages::RENDER_ATTACHMENT;
            let handle = app.world_mut().resource_mut::<Assets<Image>>().add(image);
            RenderTarget::from(handle)
        };
        app.world_mut()
            .spawn((Camera2d, target, Transform::IDENTITY));
    }
}

fn extent(width: u32, height: u32) -> Extent3d {
    Extent3d {
        width,
        height,
        depth_or_array_layers: 1,
    }
}
