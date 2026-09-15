// wgpu's type graph is deep enough that auto-trait resolution for
// `RenderDevice: Resource` blows the default limit.
#![recursion_limit = "256"]

//! E10 — does a swapped world render at all?
//!
//! The plan's version of this experiment is "two worlds, each with a camera
//! targeting its own `Image`, swap per frame, draw both images to the screen".
//! Getting there turns out to require answering Phase 2's E20 first, so this
//! binary is staged and each stage prints a result even when a later one stops:
//!
//!   1. what the render stack installs in the *main* world
//!   2. how a bare card world differs from it — the coupling report
//!   3. whether a card world survives being swapped in under the renderer
//!   4. whether two swapped worlds each render to their own image
//!
//! Stage 2 is the prototype of the thing the crate would hand a user whose
//! plugin will not start in a card world, so its shape matters more than its
//! size.

use std::collections::BTreeSet;

use bevy::prelude::*;
use bevy::render::RenderPlugin;
use bevy::render::renderer::RenderDevice;
use bevy::window::ExitCondition;

use bevy_devcards_experiments::card::{CardConfig, FIXED_STEP, card_sub_app, shared_registry};
use bevy_devcards_experiments::{BEVY_VERSION, PROFILE};

fn main() {
    println!("E10 — does a swapped world render at all?");
    println!("bevy {BEVY_VERSION}  profile={PROFILE}");
    println!();

    // ---------------------------------------------------------------------
    // Stage 1 — what the render stack puts in the MAIN world
    // ---------------------------------------------------------------------
    // This is the crux the plan's framing glosses over. "Render world, device,
    // and asset storage are shared by construction because it's the same App"
    // is true of the render world and the device. It is not true of asset
    // storage: `Assets<Image>` and `AssetServer` are ordinary resources in the
    // main world, and a whole-world swap takes them with it.
    let mut host = App::new();
    host.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: None,
                exit_condition: ExitCondition::DontExit,
                ..default()
            })
            .set(RenderPlugin {
                // Deterministic first frame: no pipeline still compiling when
                // the swap happens.
                synchronous_pipeline_compilation: true,
                ..default()
            }),
    );
    host.finish();
    host.cleanup();

    let has_device = host.world().get_resource::<RenderDevice>().is_some();
    println!("stage 1 — host app built, render device present: {has_device}");
    if !has_device {
        println!("  no RenderDevice — no GPU or no working Vulkan ICD. Stages 3-4");
        println!("  cannot run; stages 1-2 still report.");
    }

    let host_resources = resource_names(host.world());
    println!("  main-world resources after the render stack: {}", host_resources.len());
    println!();

    // ---------------------------------------------------------------------
    // Stage 2 — the coupling report
    // ---------------------------------------------------------------------
    let registry = shared_registry();
    let card = card_sub_app(&registry, CardConfig::new("card0", 1), FIXED_STEP);
    let card_resources = resource_names(card.world());

    let missing: Vec<&String> = host_resources.difference(&card_resources).collect();
    println!("stage 2 — coupling report");
    println!(
        "  a bare card world has {} resources; the host has {}",
        card_resources.len(),
        host_resources.len()
    );
    println!(
        "  {} resources exist in the host world and not in a card world:",
        missing.len()
    );
    for name in &missing {
        println!("    {name}");
    }
    println!();
    println!("  This list is the prototype coupling report. Every entry is either");
    println!("  something the card world must be given, or something that must");
    println!("  travel with the App across a swap.");
    println!();

    println!("E10 stages 1-2 complete. Stages 3-4 pending — see RESULTS.md.");
}

fn resource_names(world: &World) -> BTreeSet<String> {
    world
        .iter_resources()
        .map(|(info, _)| info.name().to_string())
        .collect()
}
