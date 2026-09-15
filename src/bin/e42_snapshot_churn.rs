//! E42 — snapshot reviewability.
//!
//! The plan's question: add a component to an unrelated entity in the card, and
//! see how much the golden file churns. If a one-line intent produces a
//! 400-line diff, a whole-world text golden is not reviewable in a pull request
//! and the crate needs a declared subset instead.
//!
//! Three changes are measured, because they are not equally cheap:
//!   A. add a component to one existing entity
//!   B. change one component's value on one entity
//!   C. spawn one extra entity at startup, before everything else

use std::process::ExitCode;

use bevy::ecs::prelude::*;

use bevy_devcards_experiments::card::{Age, CardConfig, Selected, run_card, shared_registry};
use bevy_devcards_experiments::diff::diff;
use bevy_devcards_experiments::snapshot::{Fingerprint, snapshot};
use bevy_devcards_experiments::{BEVY_VERSION, PROFILE};

const FRAME: u64 = 600;
/// Churn above this fraction of the file means "nobody reads this diff".
const REVIEWABLE_CHURN_PERCENT: f64 = 10.0;

fn main() -> ExitCode {
    println!("E42 — snapshot reviewability");
    println!("bevy {BEVY_VERSION}  profile={PROFILE}  frame={FRAME}");
    println!();

    let registry = shared_registry();
    let baseline = snapshot(run_card(&registry, CardConfig::new("churn", 7), FRAME).world());
    println!("baseline: {}", Fingerprint::of(&baseline));
    println!();

    // --- A: add a component to one existing entity -------------------------
    let a = {
        let mut sub = run_card(&registry, CardConfig::new("churn", 7), FRAME);
        let target = middle_entity(sub.world_mut());
        sub.world_mut().entity_mut(target).insert(Selected);
        snapshot(sub.world())
    };
    let stat_a = diff(&baseline, &a);
    println!("A. add `Selected` to one existing entity");
    println!("   {stat_a}");

    // --- B: change one component value -------------------------------------
    let b = {
        let mut sub = run_card(&registry, CardConfig::new("churn", 7), FRAME);
        let target = middle_entity(sub.world_mut());
        sub.world_mut()
            .entity_mut(target)
            .get_mut::<Age>()
            .unwrap()
            .ticks += 1;
        snapshot(sub.world())
    };
    let stat_b = diff(&baseline, &b);
    println!("B. change one `Age` value on one entity");
    println!("   {stat_b}");

    // --- C: spawn one extra entity at startup ------------------------------
    let c = {
        let mut cfg = CardConfig::new("churn", 7);
        cfg.extra_startup_entity = true;
        snapshot(run_card(&registry, cfg, FRAME).world())
    };
    let stat_c = diff(&baseline, &c);
    println!("C. spawn one extra entity before the sim starts");
    println!("   {stat_c}");
    println!();

    // --- verdict ------------------------------------------------------------
    println!("reviewability threshold: {REVIEWABLE_CHURN_PERCENT:.0}% of the file");
    for (name, stat) in [("A", stat_a), ("B", stat_b), ("C", stat_c)] {
        println!(
            "  {name}: {:>6.2}%  {}",
            stat.churn_percent(),
            if stat.churn_percent() <= REVIEWABLE_CHURN_PERCENT {
                "reviewable"
            } else {
                "NOT reviewable as a whole-world text golden"
            }
        );
    }
    println!();

    // Written out so the shape of the diff can be eyeballed, not just counted.
    let dir = std::path::Path::new("target/e42");
    std::fs::create_dir_all(dir).ok();
    for (name, body) in [
        ("baseline", &baseline),
        ("a_added_component", &a),
        ("b_changed_value", &b),
        ("c_extra_entity", &c),
    ] {
        std::fs::write(dir.join(format!("{name}.scn.ron")), body).ok();
    }
    println!("samples written to target/e42/ — `diff -u` them to see the shape");
    println!();

    // This experiment reports a number; it does not pass or fail. The only
    // failure mode is the measurement itself being degenerate.
    let degenerate = stat_a.touched() == 0 || stat_b.touched() == 0 || stat_c.touched() == 0;
    if degenerate {
        println!("E42 FAIL — a change produced no diff at all, so the snapshot is");
        println!("           not observing what this experiment claims to measure");
        return ExitCode::FAILURE;
    }
    println!("E42 PASS (measurement only — see the percentages above for the verdict)");
    ExitCode::SUCCESS
}

/// Pick a stable entity from the middle of the id range, so the change lands in
/// the body of the file rather than at either end.
fn middle_entity(world: &mut World) -> Entity {
    let mut ids: Vec<Entity> = world
        .iter_entities()
        .filter(|e| e.contains::<Age>())
        .map(|e| e.id())
        .collect();
    ids.sort_by_key(|e| e.index());
    ids[ids.len() / 2]
}
