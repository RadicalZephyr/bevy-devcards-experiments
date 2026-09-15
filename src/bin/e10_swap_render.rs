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

use bevy::diagnostic::FrameCount;
use bevy::prelude::*;
use bevy::render::RenderApp;
use bevy::render::RenderPlugin;
use bevy::render::renderer::{
    RenderAdapter, RenderAdapterInfo, RenderDevice, RenderInstance, RenderQueue,
};
use bevy::render::settings::RenderCreation;
use bevy::shader::Shader;
use bevy::time::TimeReceiver;
use bevy::window::ExitCondition;

use bevy_devcards_experiments::card::{CardConfig, FIXED_STEP, card_sub_app, shared_registry};
use bevy_devcards_experiments::swap::{CardWorlds, MigrationSet};
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
    println!(
        "  main-world resources after the render stack: {}",
        host_resources.len()
    );
    println!();

    // ---------------------------------------------------------------------
    // Stage 2 — the coupling report
    // ---------------------------------------------------------------------
    let registry = shared_registry();
    let card = card_sub_app(&registry, CardConfig::new("card0", 1), FIXED_STEP);
    let card_resources = resource_names(card.world());

    let missing: Vec<&String> = host_resources.difference(&card_resources).collect();
    // `Messages<T>` are double-buffered event queues. A card world gets its own
    // simply by adding the same plugins, so they inflate the number without
    // telling you anything. Split them out: the shape matters more than the size.
    let (queues, state): (Vec<&&String>, Vec<&&String>) =
        missing.iter().partition(|n| n.contains("Messages<"));

    println!("stage 2 — coupling report");
    println!(
        "  bare card world: {} resources.  host world: {} resources.",
        card_resources.len(),
        host_resources.len()
    );
    println!(
        "  {} exist in the host and not in a card: {} event queues, {} real state.",
        missing.len(),
        queues.len(),
        state.len()
    );
    println!();
    println!("  The {} that are actual state, by crate:", state.len());
    let mut by_crate: std::collections::BTreeMap<&str, Vec<&str>> = Default::default();
    for name in &state {
        let krate = name.split("::").next().unwrap_or("?");
        by_crate.entry(krate).or_default().push(name.as_str());
    }
    for (krate, names) in &by_crate {
        println!("    {krate} ({})", names.len());
        for n in names {
            println!("      {}", short(n));
        }
    }
    println!();
    println!("  The headline: `AssetServer` and every `Assets<T>` are in this list.");
    println!("  The plan assumes asset storage is shared by construction because it");
    println!("  is the same App. It is not — asset storage is a set of ordinary main");
    println!("  world resources, and a whole-world swap takes them with it.");
    println!();

    // ---------------------------------------------------------------------
    // Stage 3 — does a swapped-in card world survive one frame?
    // ---------------------------------------------------------------------
    // Migrate everything the public API lets us name. If the swap still fails
    // with a complete public migration set, the failure is structural rather
    // than a matter of finding one more resource.
    let migration = public_migration_set();
    println!("stage 3 — swapping a card world in under the renderer");
    println!(
        "  migration set: {} resources, every one of them publicly nameable",
        migration.len()
    );

    let mut card_world = card;
    let card_world = core::mem::take(card_world.world_mut());
    let mut cards = CardWorlds::new(vec![card_world], migration);
    let absent = cards.swap_to(host.world_mut(), 0);
    if absent.is_empty() {
        println!("  swap done: every declared resource was found and moved");
    } else {
        println!(
            "  swap done, but {} declared resources were absent:",
            absent.len()
        );
        for name in &absent {
            println!("    {}", short(name));
        }
    }

    // The swapped-in world is missing whatever the migration set could not name.
    let after = resource_names(host.world());
    let still_missing: Vec<&String> = host_resources
        .difference(&after)
        .filter(|n| !n.contains("Messages<"))
        .collect();
    println!(
        "  after the swap the App's world is missing {} of the host's non-queue resources",
        still_missing.len()
    );

    println!();
    println!("  ticking the App once...");
    let panic_message = capture_panic(|| host.update());
    match panic_message {
        None => {
            println!("  SURVIVED. A bare card world ticks under the renderer.");
            println!("  -> proceed to stage 4 (two worlds, two images).");
        }
        Some(message) => {
            println!("  PANICKED: {message}");
            println!();
            println!("  The blocker: three main-world resources the swap must carry are");
            println!("  not reachable from outside bevy_render:");
            println!("    bevy_render::extract_plugin::ScratchMainWorld        (private)");
            println!("    bevy_render::sync_world::PendingSyncEntity           (pub(crate))");
            println!(
                "    bevy_render::render_asset::CachedExtractRenderAssetSystemState<A> (private)"
            );
        }
    }
    println!();

    // ---------------------------------------------------------------------
    // Stage 3b — build the card world through the same plugin stack
    // ---------------------------------------------------------------------
    // If a card world cannot be *given* the private resources, it can build
    // them itself: run the same `RenderPlugin` over it, with
    // `RenderCreation::Manual` borrowing the host's device so no second GPU
    // context is created. `ExtractPlugin::build` then installs
    // `ScratchMainWorld` and `PendingSyncEntity` into the card world directly.
    //
    // Note where this mechanism comes from: it is Phase 3's E30
    // (`RenderCreation::Manual` on a borrowed device), used as a component of
    // Phase 1 rather than as its fallback.
    println!("stage 3b — card world built through the same plugin stack");
    let mut host2 = build_host();
    let Some(borrowed) = borrow_render_resources(&host2) else {
        println!("  no render resources to borrow; stopping.");
        return;
    };

    let mut built = Vec::new();
    for i in 0..CARDS_3B {
        match capture_panic_value(|| card_world_with_render_stack(&borrowed)) {
            Ok(world) => built.push(world),
            Err(message) => {
                println!("  PANICKED building card {i}: {message}");
                return;
            }
        }
    }
    println!("  {CARDS_3B} card worlds built on the host's borrowed device");

    let names = resource_names(&built[0]);
    for probe in [
        "bevy_render::sync_world::PendingSyncEntity",
        "bevy_render::extract_plugin::ScratchMainWorld",
    ] {
        println!(
            "  card world has {}: {}",
            short(probe),
            names.contains(probe)
        );
    }

    // The card brought its own copies of the private resources, so the
    // migration set only has to carry things that must be *shared* with the
    // host's render world rather than duplicated. `TimeReceiver` is one: the
    // host's render app holds the matching `TimeSender`, and a card with its
    // own channel leaves the host's sender writing into a queue nobody drains.
    let migration2 = MigrationSet::new().with::<TimeReceiver>();
    let mut cards2 = CardWorlds::new(built, migration2);
    println!(
        "  migration set: {} resource(s) — {}",
        cards2.migration().len(),
        cards2.migration().names().join(", ")
    );

    println!("  round-robin swapping for {FRAMES_3B} frames...");
    let result = capture_panic(|| {
        for _ in 0..FRAMES_3B {
            cards2.swap_next(host2.world_mut());
            host2.update();
        }
    });
    match result {
        None => {
            println!(
                "  SURVIVED {} swaps across {CARDS_3B} worlds.",
                cards2.swaps()
            );
            println!("  Gate 1 is NOT blocked by resource visibility. The naive swap is,");
            println!("  but a card world built through the same plugin stack is not.");
            println!("  -> next: stage 4, verify each card's image actually updates.");
        }
        Some(message) => {
            println!("  PANICKED after {} swaps: {message}", cards2.swaps());
        }
    }
}

const CARDS_3B: usize = 3;
const FRAMES_3B: usize = 240;

/// The host App: everything, including the renderer, and no window.
fn build_host() -> App {
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

/// The five handles `RenderCreation::manual` needs, all publicly nameable.
struct BorrowedRenderResources {
    device: RenderDevice,
    queue: RenderQueue,
    adapter_info: RenderAdapterInfo,
    adapter: RenderAdapter,
    instance: RenderInstance,
}

/// Note the asymmetry, which the coupling report already showed: `RenderDevice`,
/// `RenderQueue`, `RenderAdapter` and `RenderAdapterInfo` are inserted into both
/// worlds, but `RenderInstance` goes only into the render world. Borrowing the
/// full set needs both.
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

fn card_world_with_render_stack(res: &BorrowedRenderResources) -> World {
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: None,
                exit_condition: ExitCondition::DontExit,
                ..default()
            })
            .set(RenderPlugin {
                render_creation: RenderCreation::manual(
                    res.device.clone(),
                    res.queue.clone(),
                    res.adapter_info.clone(),
                    res.adapter.clone(),
                    res.instance.clone(),
                ),
                synchronous_pipeline_compilation: true,
                ..default()
            }),
    );
    app.finish();
    app.cleanup();
    core::mem::take(app.world_mut())
}

fn capture_panic_value<T>(f: impl FnOnce() -> T) -> Result<T, String> {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::sync::{Arc, Mutex};

    let captured: Arc<Mutex<Option<String>>> = Arc::default();
    let sink = Arc::clone(&captured);
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        *sink.lock().unwrap() = Some(info.to_string());
    }));
    let result = catch_unwind(AssertUnwindSafe(f));
    std::panic::set_hook(previous);

    result.map_err(|_| {
        captured
            .lock()
            .unwrap()
            .take()
            .unwrap_or_else(|| "panicked, message not captured".to_string())
    })
}

/// Everything in the coupling report that a third-party crate can actually name.
fn public_migration_set() -> MigrationSet {
    MigrationSet::new()
        .with::<AssetServer>()
        .with::<Assets<Image>>()
        .with::<Assets<Mesh>>()
        .with::<Assets<Shader>>()
        .with::<RenderDevice>()
        .with::<RenderQueue>()
        .with::<RenderAdapter>()
        .with::<RenderAdapterInfo>()
        .with::<ClearColor>()
        .with::<FrameCount>()
}

/// Trim the crate prefix off a type path so the report is readable.
fn short(name: &str) -> String {
    name.replace("bevy_ecs::message::messages::", "")
        .replace("bevy_asset::assets::", "")
        .replace("bevy_render::renderer::render_device::", "")
        .replace("bevy_render::renderer::", "")
        .replace("bevy_render::", "")
        .replace("bevy_asset::", "")
}

fn capture_panic(f: impl FnOnce()) -> Option<String> {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::sync::{Arc, Mutex};

    let captured: Arc<Mutex<Option<String>>> = Arc::default();
    let sink = Arc::clone(&captured);
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        *sink.lock().unwrap() = Some(info.to_string());
    }));
    let result = catch_unwind(AssertUnwindSafe(f));
    std::panic::set_hook(previous);

    match result {
        Ok(()) => None,
        Err(_) => Some(
            captured
                .lock()
                .unwrap()
                .take()
                .unwrap_or_else(|| "panicked, message not captured".to_string()),
        ),
    }
}

fn resource_names(world: &World) -> BTreeSet<String> {
    world
        .iter_resources()
        .map(|(info, _)| info.name().to_string())
        .collect()
}
