//! E13 — dormancy.
//!
//! "A world that skips 1000 frames and then resumes." The plan calls this a
//! sleeper, and it is right to: a card that behaves differently after being
//! backgrounded is worse than a card that doesn't render, and nothing about it
//! is visible until someone leaves a card off screen for a minute.
//!
//! In the architecture that came out of Phase 3, a dormant card is an App whose
//! `update()` simply is not called. So the question is really: **can anything
//! leak into a world nobody is ticking?** Real time, a shared registry, a
//! global tick counter, an event buffer someone else drains — any of those would
//! make dormancy observable.
//!
//! Five checks. The first is the headline and the other four say why it holds.

use std::time::{Duration, Instant};

use bevy::app::{Main, MainSchedulePlugin, SubApp, Update};
use bevy::ecs::change_detection::CHECK_TICK_THRESHOLD;
use bevy::ecs::prelude::*;
use bevy::ecs::schedule::{IntoScheduleConfigs, ScheduleLabel};
use bevy::time::{Time, Virtual};

use bevy_devcards_experiments::card::{
    CardConfig, CardStep, FIXED_STEP, card_sub_app, shared_registry,
};
use bevy_devcards_experiments::snapshot::{Fingerprint, snapshot};
use bevy_devcards_experiments::{BEVY_VERSION, PROFILE};

const TICKS_BEFORE: u64 = 300;
const TICKS_AFTER: u64 = 300;
/// How many frames the rest of the process runs while one card sleeps.
const DORMANT_FRAMES: u64 = 1000;
/// Real time that passes during dormancy. Nothing in a card should notice.
const DORMANT_REAL_TIME: Duration = Duration::from_millis(150);

fn main() {
    println!("E13 — dormancy");
    println!("bevy {BEVY_VERSION}  profile={PROFILE}");
    println!(
        "a card sleeps for {DORMANT_FRAMES} frames and {} ms of real time",
        DORMANT_REAL_TIME.as_millis()
    );
    println!();

    let mut ok = true;
    ok &= check_equivalence();
    ok &= check_probe_world();
    ok &= check_tick_wraparound();

    println!();
    println!("{}", if ok { "E13 PASS" } else { "E13 FAIL" });
    if !ok {
        std::process::exit(1);
    }
}

// ---------------------------------------------------------------------------
// A — behavioural equivalence
// ---------------------------------------------------------------------------

/// The headline check: a card interrupted by a long dormancy must end up in
/// exactly the state it would have reached without one.
///
/// This is only meaningful because the dormancy is *realistic* — other cards
/// tick thousands of times and real time passes while this one sleeps. If any
/// global state leaked into a card world, the two snapshots would diverge.
fn check_equivalence() -> bool {
    println!("A. dormant vs continuous — byte-identical state?");
    let registry = shared_registry();

    let continuous = {
        let mut card = card_sub_app(&registry, CardConfig::new("dormancy", 11), FIXED_STEP);
        for _ in 0..(TICKS_BEFORE + TICKS_AFTER) {
            card.update();
        }
        snapshot(card.world())
    };

    let dormant = {
        let mut card = card_sub_app(&registry, CardConfig::new("dormancy", 11), FIXED_STEP);
        let mut neighbours: Vec<SubApp> = (0..2)
            .map(|i| card_sub_app(&registry, CardConfig::new("neighbour", 500 + i), FIXED_STEP))
            .collect();

        for _ in 0..TICKS_BEFORE {
            card.update();
        }

        // Dormancy: the card is not ticked at all, while the process very much
        // keeps going.
        let slept = Instant::now();
        for _ in 0..DORMANT_FRAMES {
            for neighbour in &mut neighbours {
                neighbour.update();
            }
        }
        let remaining = DORMANT_REAL_TIME.saturating_sub(slept.elapsed());
        if !remaining.is_zero() {
            std::thread::sleep(remaining);
        }

        for _ in 0..TICKS_AFTER {
            card.update();
        }
        snapshot(card.world())
    };

    println!("   continuous: {}", Fingerprint::of(&continuous));
    println!("   dormant:    {}", Fingerprint::of(&dormant));
    let same = continuous == dormant;
    println!(
        "   {}",
        if same {
            "PASS: dormancy is not observable in the card's state"
        } else {
            "FAIL: the card ended up somewhere else for having been backgrounded"
        }
    );
    println!();
    same
}

// ---------------------------------------------------------------------------
// B, C, D — a purpose-built probe world
// ---------------------------------------------------------------------------

#[derive(Component)]
struct Stable;

#[derive(Component)]
struct Counter(u32);

#[derive(Message)]
struct Ping;

#[derive(Resource, Default, Debug)]
struct Observations {
    ticks: u32,
    /// Entities a `Changed<Stable>` query saw on the most recent tick. `Stable`
    /// is never mutated, so in steady state this must be zero.
    stable_changed_last: usize,
    stable_added_last: usize,
    pings_read_total: usize,
    last_delta: Duration,
}

fn observe(
    stable_changed: Query<(), Changed<Stable>>,
    stable_added: Query<(), Added<Stable>>,
    mut pings: MessageReader<Ping>,
    time: Res<Time>,
    mut obs: ResMut<Observations>,
) {
    obs.ticks += 1;
    obs.stable_changed_last = stable_changed.iter().count();
    obs.stable_added_last = stable_added.iter().count();
    obs.pings_read_total += pings.read().count();
    obs.last_delta = time.delta();
}

fn advance_time(step: Res<CardStep>, mut virt: ResMut<Time<Virtual>>, mut generic: ResMut<Time>) {
    virt.advance_by(step.0);
    *generic = virt.as_generic();
}

fn bump(mut q: Query<&mut Counter>) {
    for mut c in &mut q {
        c.0 += 1;
    }
}

fn probe_world() -> SubApp {
    let mut sub = SubApp::new();
    sub.update_schedule = Some(Main.intern());
    sub.add_plugins(MainSchedulePlugin);
    sub.init_resource::<Time>();
    sub.init_resource::<Time<Virtual>>();
    sub.insert_resource(CardStep(FIXED_STEP));
    sub.init_resource::<Observations>();
    sub.add_message::<Ping>();
    for i in 0..16 {
        sub.world_mut().spawn((Stable, Counter(i)));
    }
    sub.add_systems(Update, (advance_time, bump, observe).chain());
    sub
}

fn check_probe_world() -> bool {
    println!("B/C/D. change detection, time, and messages across a sleep");
    let mut probe = probe_world();

    for _ in 0..TICKS_BEFORE {
        probe.update();
    }
    let before = format!("{:?}", probe.world().resource::<Observations>());
    let ticks_before = probe.world().resource::<Observations>().ticks;
    let elapsed_before = probe.world().resource::<Time<Virtual>>().elapsed();

    // A message written on the last tick before sleeping. Messages are
    // double-buffered, so this is the interesting case: it has been written but
    // not yet aged out.
    probe
        .world_mut()
        .resource_mut::<Messages<Ping>>()
        .write(Ping);
    let queued = probe.world().resource::<Messages<Ping>>().len();

    // Sleep.
    std::thread::sleep(DORMANT_REAL_TIME);
    let queued_after_sleep = probe.world().resource::<Messages<Ping>>().len();

    probe.update();
    let obs = probe.world().resource::<Observations>();
    let pings_after_resume = obs.pings_read_total;
    let delta_after_resume = obs.last_delta;
    let stable_changed = obs.stable_changed_last;
    let stable_added = obs.stable_added_last;
    let elapsed_after = probe.world().resource::<Time<Virtual>>().elapsed();

    let mut ok = true;

    // B — change detection
    println!("   B. change detection");
    println!("      Changed<Stable> on the resume tick: {stable_changed} (expected 0)");
    println!("      Added<Stable>   on the resume tick: {stable_added} (expected 0)");
    if stable_changed != 0 || stable_added != 0 {
        println!("      FAIL: a never-touched component looked new after a sleep");
        ok = false;
    } else {
        println!("      PASS: nothing spuriously read as changed");
    }

    // C — time
    println!("   C. time");
    println!("      delta on the resume tick: {delta_after_resume:?}");
    println!("      expected: {FIXED_STEP:?}");
    let expected_elapsed = FIXED_STEP * (ticks_before as u32 + 1);
    println!("      virtual elapsed: {elapsed_after:?} (expected {expected_elapsed:?})");
    if delta_after_resume != FIXED_STEP || elapsed_after != expected_elapsed {
        println!("      FAIL: the real-time gap reached the card's clock");
        ok = false;
    } else {
        println!(
            "      PASS: {} ms of real time was invisible",
            DORMANT_REAL_TIME.as_millis()
        );
        let _ = elapsed_before;
    }

    // D — messages
    println!("   D. messages");
    println!("      queued before the sleep: {queued}");
    println!("      queued after the sleep:  {queued_after_sleep} (must not grow or drop)");
    println!("      read on the resume tick: {pings_after_resume} (expected 1)");
    if queued_after_sleep != queued {
        println!("      FAIL: the message buffer changed while nothing was running");
        ok = false;
    } else if pings_after_resume != 1 {
        println!("      FAIL: the message did not survive the sleep intact");
        ok = false;
    } else {
        println!("      PASS: messages are paused, not dropped and not accumulating");
    }

    let _ = before;
    println!();
    ok
}

// ---------------------------------------------------------------------------
// E — change-tick wraparound
// ---------------------------------------------------------------------------

/// The plan asks whether `check_change_ticks` misbehaves on a world that has not
/// been ticked in a long time. Bevy only runs that check from `Schedule::run`,
/// gated on the world's *own* counters, so a dormant world never runs it at all
/// — but that is an argument, not a measurement.
///
/// This measures it. `CHECK_TICK_THRESHOLD` is 518.4 million ticks, which at
/// 60fps is about one hundred days of running, but `increment_change_tick` is
/// public and cheap enough to get there in about a second.
fn check_tick_wraparound() -> bool {
    println!("E. change-tick wraparound");
    println!("   driving a world past CHECK_TICK_THRESHOLD ({CHECK_TICK_THRESHOLD} ticks)");

    let mut probe = probe_world();
    for _ in 0..8 {
        probe.update();
    }
    let baseline_changed = probe.world().resource::<Observations>().stable_changed_last;

    let tick_before = probe.world().read_change_tick().get();
    let start = Instant::now();
    {
        let world = probe.world_mut();
        for _ in 0..(CHECK_TICK_THRESHOLD as u64 + 1024) {
            world.increment_change_tick();
        }
    }
    let tick_after = probe.world().read_change_tick().get();
    // The loop collapses to a single add in release, so the elapsed time says
    // nothing. The tick values are the evidence that the counter really moved.
    println!(
        "   change tick {tick_before} -> {tick_after} (+{}) in {:?}",
        tick_after.wrapping_sub(tick_before),
        start.elapsed()
    );

    let fired = probe.world_mut().check_change_ticks().is_some();
    println!("   check_change_ticks fired: {fired}");
    if !fired {
        println!("   FAIL: the threshold was not actually crossed; probe is inconclusive");
        return false;
    }

    probe.update();
    let obs = probe.world().resource::<Observations>();
    println!(
        "   Changed<Stable> after the clamp: {} (expected {baseline_changed})",
        obs.stable_changed_last
    );

    let ok = obs.stable_changed_last == baseline_changed;
    println!(
        "   {}",
        if ok {
            "PASS: clamping old ticks did not make untouched components look new"
        } else {
            "FAIL: components spuriously read as changed after the tick check"
        }
    );
    ok
}
