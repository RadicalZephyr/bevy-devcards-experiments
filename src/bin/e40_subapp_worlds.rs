//! E40 — N card worlds as SubApps, headless.
//!
//! Question from the plan: do card worlds work as sub-apps, ticked with their
//! own `Time<Virtual>` at a fixed step, with nothing rendering?
//!
//! This also picks up two answers for free that Phase 1 will need: the order
//! sub-apps are ticked in, and whether entity ids collide across worlds (E12's
//! hazard, visible here at zero cost).

use std::time::Instant;

use bevy::app::{App, AppLabel};
use bevy::ecs::prelude::*;
use bevy::time::{Time, Virtual};

use bevy_devcards_experiments::card::{
    Age, CardClock, CardConfig, FIXED_STEP, Position, card_sub_app, shared_registry,
};
use bevy_devcards_experiments::{BEVY_VERSION, PROFILE};

#[derive(AppLabel, Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct Card(usize);

const CARDS: usize = 8;
const FRAMES: u64 = 600;

fn main() {
    println!("E40 — N card worlds as SubApps");
    println!("bevy {BEVY_VERSION}  profile={PROFILE}  cards={CARDS}  frames={FRAMES}");
    println!();

    let registry = shared_registry();
    let mut app = App::new();

    // Each card gets its own seed and its own fixed step, so "card 3 ran at a
    // different rate than card 5" is checkable rather than assumed.
    for i in 0..CARDS {
        let cfg = CardConfig::new(&format!("card{i}"), 1000 + i as u64);
        let step = FIXED_STEP * (1 + i as u32 % 3);
        app.insert_sub_app(Card(i), card_sub_app(&registry, cfg, step));
    }

    app.finish();
    app.cleanup();

    // The order `SubApps::update` visits sub-apps in is HashMap iteration order,
    // not insertion order. Record it: a card that quietly depends on being
    // ticked before another card would be relying on this.
    let visit_order: Vec<String> = app
        .sub_apps()
        .sub_apps
        .keys()
        .map(|label| format!("{label:?}"))
        .collect();

    let start = Instant::now();
    for _ in 0..FRAMES {
        app.update();
    }
    let elapsed = start.elapsed();

    // --- per-card state -----------------------------------------------------
    println!(
        "{:<8} {:>6} {:>10} {:>12} {:>14}",
        "card", "ticks", "particles", "virtual_s", "step_ms"
    );
    let mut ok = true;
    for i in 0..CARDS {
        let sub = app.sub_app(Card(i));
        let world = sub.world();
        let clock = world.resource::<CardClock>();
        let virt = world.resource::<Time<Virtual>>();
        let particles = count_particles(sub.world());
        let step = FIXED_STEP * (1 + i as u32 % 3);

        println!(
            "{:<8} {:>6} {:>10} {:>12.4} {:>14.4}",
            format!("card{i}"),
            clock.tick,
            particles,
            virt.elapsed_secs_f64(),
            step.as_secs_f64() * 1000.0,
        );

        // Every card ticked exactly once per host frame.
        if clock.tick != FRAMES {
            println!(
                "  FAIL: card{i} ticked {} times, expected {FRAMES}",
                clock.tick
            );
            ok = false;
        }
        // And its virtual clock advanced by its own step, not the host's.
        let expected = step.as_secs_f64() * FRAMES as f64;
        if (virt.elapsed_secs_f64() - expected).abs() > 1e-9 {
            println!(
                "  FAIL: card{i} virtual elapsed {} != {expected}",
                virt.elapsed_secs_f64()
            );
            ok = false;
        }
    }
    println!();

    // --- isolation ----------------------------------------------------------
    // Same tick count, different seeds: if state were shared, these would agree.
    let a = first_positions(app.sub_app(Card(0)).world(), 4);
    let b = first_positions(app.sub_app(Card(3)).world(), 4);
    println!("card0 first positions: {a:?}");
    println!("card3 first positions: {b:?}");
    if a == b {
        println!("  FAIL: two differently seeded cards produced identical state");
        ok = false;
    } else {
        println!("  PASS: differently seeded cards diverge");
    }
    println!();

    // --- entity id collision across worlds (E12's hazard, seen early) -------
    let shared = shared_entity_ids(app.sub_app(Card(0)).world(), app.sub_app(Card(1)).world());
    println!(
        "entity ids live in BOTH card0 and card1: {} (of {} / {})",
        shared,
        count_particles(app.sub_app(Card(0)).world()),
        count_particles(app.sub_app(Card(1)).world()),
    );
    println!(
        "  every isolated world allocates from index 0, so `MainEntity(Entity)` \
         is NOT a unique key across cards."
    );
    println!("  -> Phase 1 E12 has to offset allocators or the render world will alias.");
    println!();

    // --- ordering -----------------------------------------------------------
    println!("sub-app visit order: {}", visit_order.join(", "));
    println!("  (HashMap order over `InternedAppLabel`, which hashes by pointer)");
    // Interning is process-global, so the order cannot vary *within* a run — a
    // second App with the same labels always agrees. It varies between runs,
    // because the interner's allocations land at different addresses. Compare
    // against the previous run's order on disk; CI runs this binary twice so
    // the evidence lands in the log rather than in a claim.
    let order_path = std::path::Path::new("target/e40_visit_order.txt");
    let current = visit_order.join(", ");
    match std::fs::read_to_string(order_path) {
        Ok(previous) if previous.trim() == current => {
            println!("  same order as the previous run of this binary");
        }
        Ok(previous) => {
            println!("  previous run:   {}", previous.trim());
            println!("  DIFFERENT between runs of the same binary. Sub-app tick order is");
            println!("  not merely unspecified, it is unstable: a card that depends on");
            println!("  running before or after another is nondeterministic.");
        }
        Err(_) => println!("  (no previous run recorded; run again to compare)"),
    }
    std::fs::create_dir_all("target").ok();
    std::fs::write(order_path, &current).ok();
    println!();

    // --- throughput ---------------------------------------------------------
    let per_frame_ms = elapsed.as_secs_f64() * 1000.0 / FRAMES as f64;
    println!(
        "{CARDS} cards x {FRAMES} frames in {:.1} ms -> {:.4} ms/frame, {:.4} ms/card/tick",
        elapsed.as_secs_f64() * 1000.0,
        per_frame_ms,
        per_frame_ms / CARDS as f64,
    );
    println!(
        "  extrapolated cards tickable in a 16.67 ms budget: {:.0}",
        16.667 / (per_frame_ms / CARDS as f64),
    );
    println!();

    println!("{}", if ok { "E40 PASS" } else { "E40 FAIL" });
    if !ok {
        std::process::exit(1);
    }
}

fn count_particles(world: &World) -> usize {
    world
        .iter_entities()
        .filter(|e| e.contains::<Age>())
        .count()
}

fn first_positions(world: &World, n: usize) -> Vec<(f32, f32)> {
    let mut rows: Vec<_> = world
        .iter_entities()
        .filter_map(|e| {
            e.get::<Position>()
                .map(|p| (e.id().index(), (p.0.x, p.0.y)))
        })
        .collect();
    rows.sort_by_key(|(index, _)| *index);
    rows.into_iter().take(n).map(|(_, p)| p).collect()
}

/// How many raw `Entity` bit patterns are live in both worlds at once.
fn shared_entity_ids(a: &World, b: &World) -> usize {
    let ids_b: Vec<Entity> = b
        .iter_entities()
        .filter(|e| e.contains::<Age>())
        .map(|e| e.id())
        .collect();
    a.iter_entities()
        .filter(|e| e.contains::<Age>())
        .filter(|e| ids_b.contains(&e.id()))
        .count()
}
