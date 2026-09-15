//! E00 — headless world tick cost.
//!
//! "An empty `World` with a minimal schedule, ticked in a loop. How many can you
//! tick per 16ms?" This sets the ceiling on simultaneous live cards, and
//! therefore on whether "many states at once" means six or sixty.
//!
//! Measured two ways on purpose, because they are not the same question:
//!
//!   - **one world, many ticks** — the plan's literal wording, and the number
//!     that isolates schedule overhead.
//!   - **N worlds, one tick each, round-robin** — what cards actually do. Every
//!     pass touches N separate archetype stores, so if there is a cache cost to
//!     isolation it shows up here and nowhere else.
//!
//! If those two diverge, the ceiling is set by the second one.

use std::time::Instant;

use bevy::app::{Main, MainSchedulePlugin, SubApp, Update};
use bevy::ecs::prelude::*;
use bevy::ecs::schedule::{IntoScheduleConfigs, ScheduleLabel};

use bevy_devcards_experiments::card::{CardConfig, FIXED_STEP, card_sub_app, shared_registry};
use bevy_devcards_experiments::{BEVY_VERSION, PROFILE};

/// One 60fps frame, in milliseconds. The budget everything is divided into.
const FRAME_BUDGET_MS: f64 = 1000.0 / 60.0;
/// Every configuration is measured over the same number of *world-ticks*, so
/// the single-world and round-robin numbers are directly comparable.
const WORLD_TICKS: usize = 40_000;
/// Best-of-N. This is a laptop; the first timed loop of the process runs at a
/// lower clock than the rest, and without this the first shape measured comes
/// out slower than shapes that do strictly more work.
const TRIALS: usize = 5;
/// How many worlds the round-robin variant spreads across.
const FANOUT: usize = 64;
/// "~200 entities", per the plan.
const ENTITIES: usize = 200;

fn main() {
    println!("E00 — headless world tick cost");
    println!("bevy {BEVY_VERSION}  profile={PROFILE}");
    if PROFILE == "debug" {
        println!();
        println!("  WARNING: debug. E40 measured release at 59x faster. Do not quote");
        println!("  these numbers for Gate 3 — rerun with --release.");
    }
    println!();

    let registry = shared_registry();

    let shapes: [(&str, &dyn Fn() -> SubApp); 3] = [
        ("empty world, 1 no-op system", &empty_world),
        ("200 entities, 5 systems", &busy_world),
        ("the Phase 4 card sim", &|| {
            card_sub_app(&registry, CardConfig::new("e00", 9), FIXED_STEP)
        }),
    ];

    println!(
        "{:<28} {:>14} {:>14} {:>12} {:>12}",
        "shape", "1 world (µs)", "64 worlds (µs)", "cards @60fps", "penalty"
    );

    // Spin the CPU up before anything is timed, for the same reason.
    spin_up();

    for (label, build) in shapes {
        let mut single = vec![build()];
        let per_tick_single = best_of(&mut single);

        let mut fanned: Vec<SubApp> = (0..FANOUT).map(|_| build()).collect();
        let per_tick_fanned = best_of(&mut fanned);

        // The round-robin number is the one that bounds the design.
        let budget = (FRAME_BUDGET_MS * 1000.0 / per_tick_fanned).floor();
        let penalty = per_tick_fanned / per_tick_single;

        println!(
            "{label:<28} {per_tick_single:>14.3} {per_tick_fanned:>14.3} {budget:>12.0} {penalty:>11.2}x"
        );
    }

    println!();
    println!("  \"cards @60fps\" uses the round-robin cost: N distinct worlds, one");
    println!("  tick each per frame, which is what a card grid actually does.");
    println!("  \"penalty\" is how much more a tick costs when it is spread across");
    println!("  {FANOUT} worlds instead of repeated on one — the cost of isolation.");
    println!();
    println!("  Gate 3 compares this against E01's thumbnail ceiling. A large gap");
    println!("  means the design is \"many simulating, few visible\".");
}

/// Tick a throwaway world until the clock has had time to ramp.
fn spin_up() {
    let mut sub = busy_world();
    let start = Instant::now();
    while start.elapsed().as_millis() < 400 {
        for _ in 0..1000 {
            sub.update();
        }
    }
}

/// Microseconds per world-tick, best of [`TRIALS`] runs.
fn best_of(subs: &mut [SubApp]) -> f64 {
    let passes = (WORLD_TICKS / subs.len()).max(1);
    // One untimed pass-set so every archetype and system state is warm.
    time_passes(subs, passes.min(200));
    (0..TRIALS)
        .map(|_| time_passes(subs, passes))
        .fold(f64::INFINITY, f64::min)
}

fn time_passes(subs: &mut [SubApp], passes: usize) -> f64 {
    let start = Instant::now();
    for _ in 0..passes {
        for sub in subs.iter_mut() {
            sub.update();
        }
    }
    start.elapsed().as_secs_f64() * 1e6 / (passes * subs.len()) as f64
}

fn new_sub_app() -> SubApp {
    let mut sub = SubApp::new();
    sub.update_schedule = Some(Main.intern());
    sub.add_plugins(MainSchedulePlugin);
    sub
}

fn empty_world() -> SubApp {
    let mut sub = new_sub_app();
    sub.add_systems(Update, || {});
    sub
}

// --- the 200-entity shape ---------------------------------------------------

#[derive(Component)]
struct Pos {
    x: f32,
    y: f32,
}

#[derive(Component)]
struct Vel {
    x: f32,
    y: f32,
}

#[derive(Component)]
struct Ticks(u32);

#[derive(Component)]
struct Heat(f32);

#[derive(Resource, Default)]
struct Accumulator(f64);

fn busy_world() -> SubApp {
    let mut sub = new_sub_app();
    sub.init_resource::<Accumulator>();
    {
        let world = sub.world_mut();
        for i in 0..ENTITIES {
            let f = i as f32;
            world.spawn((
                Pos { x: f, y: -f },
                Vel {
                    x: 1.0 + f * 0.015625,
                    y: 1.0 - f * 0.015625,
                },
                Ticks(0),
                Heat(f * 0.5),
            ));
        }
    }
    sub.add_systems(Update, (integrate, bounce, age, cool, accumulate).chain());
    sub
}

fn integrate(mut q: Query<(&mut Pos, &Vel)>) {
    for (mut p, v) in &mut q {
        p.x += v.x * 0.015625;
        p.y += v.y * 0.015625;
    }
}

fn bounce(mut q: Query<(&mut Pos, &mut Vel)>) {
    for (mut p, mut v) in &mut q {
        if p.x < -64.0 || p.x > 64.0 {
            p.x = p.x.clamp(-64.0, 64.0);
            v.x = -v.x;
        }
        if p.y < -64.0 || p.y > 64.0 {
            p.y = p.y.clamp(-64.0, 64.0);
            v.y = -v.y;
        }
    }
}

fn age(mut q: Query<&mut Ticks>) {
    for mut t in &mut q {
        t.0 += 1;
    }
}

fn cool(mut q: Query<&mut Heat>) {
    for mut h in &mut q {
        h.0 *= 0.99609375;
    }
}

fn accumulate(q: Query<(&Pos, &Heat)>, mut acc: ResMut<Accumulator>) {
    let mut sum = 0.0f64;
    for (p, h) in &q {
        sum += (p.x + h.0) as f64;
    }
    acc.0 = sum;
}
