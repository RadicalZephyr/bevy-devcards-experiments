//! E41 — snapshot determinism.
//!
//! The plan asks for a `DynamicScene` snapshot of a card world at frame N,
//! repeated 10x on the same binary, then debug vs release, then this machine vs
//! a CI runner, checking for byte-identical output.
//!
//! `DynamicScene` does not exist in Bevy 0.19 — see `snapshot.rs`. The
//! equivalent is `bevy::world_serialization::DynamicWorld`, and that is what is
//! measured here.
//!
//! Usage:
//!   e41_snapshot_determinism            compare against the committed golden
//!   e41_snapshot_determinism --write    (re)write the golden

use std::path::Path;
use std::process::ExitCode;

use bevy_devcards_experiments::card::{
    CardConfig, FIXED_STEP, card_sub_app, run_card, shared_registry,
};
use bevy_devcards_experiments::snapshot::{Fingerprint, snapshot};
use bevy_devcards_experiments::{BEVY_VERSION, PROFILE};

const FRAME: u64 = 600;
const REPEATS: usize = 10;
const GOLDEN: &str = "goldens/e41_card.scn.ron";

fn main() -> ExitCode {
    let write_golden = std::env::args().any(|a| a == "--write");

    println!("E41 — snapshot determinism");
    println!(
        "bevy {BEVY_VERSION}  profile={PROFILE}  arch={}  os={}  frame={FRAME}",
        std::env::consts::ARCH,
        std::env::consts::OS,
    );
    println!();

    let registry = shared_registry();
    let mut ok = true;

    // --- 1. same binary, same process, N times from scratch ----------------
    let runs: Vec<String> = (0..REPEATS)
        .map(|_| {
            let sub = run_card(&registry, CardConfig::new("determinism", 42), FRAME);
            snapshot(sub.world())
        })
        .collect();

    let reference = &runs[0];
    println!("run 0: {}", Fingerprint::of(reference));
    let identical = runs.iter().all(|r| r == reference);
    if identical {
        println!("  PASS: {REPEATS}/{REPEATS} in-process runs byte-identical");
    } else {
        ok = false;
        println!("  FAIL: in-process runs diverged");
        for (i, r) in runs.iter().enumerate() {
            println!("    run {i}: {}", Fingerprint::of(r));
        }
    }
    println!();

    // --- 2. does a card's snapshot depend on what else is running? ----------
    // Three cards ticked round-robin in one process, then card 0 compared
    // against the same card run alone. If isolation is real this holds, and if
    // it does not, nothing downstream in the crate is trustworthy.
    let interleaved = {
        let mut subs: Vec<_> = (0..3)
            .map(|i| {
                // Card 0 must be configured *identically* to the reference
                // card, name included: `CardConfig` is a reflected resource, so
                // a different name is a different snapshot on its own.
                let cfg = if i == 0 {
                    CardConfig::new("determinism", 42)
                } else {
                    CardConfig::new("neighbour", 900 + i)
                };
                card_sub_app(&registry, cfg, FIXED_STEP)
            })
            .collect();
        for _ in 0..FRAME {
            for sub in &mut subs {
                sub.update();
            }
        }
        snapshot(subs[0].world())
    };
    println!(
        "card 0 run alongside two other cards: {}",
        Fingerprint::of(&interleaved)
    );
    if interleaved == *reference {
        println!("  PASS: identical to the same card run in isolation");
    } else {
        ok = false;
        println!("  FAIL: a card's snapshot depends on its neighbours");
    }
    println!();

    // --- 3. different seed must produce a different snapshot ----------------
    // Guards against the snapshot being trivially constant, which would make
    // every check above pass for the wrong reason.
    let other = snapshot(run_card(&registry, CardConfig::new("determinism", 43), FRAME).world());
    if other == *reference {
        ok = false;
        println!("  FAIL: seed 43 produced the same snapshot as seed 42 — the");
        println!("        snapshot is not actually observing simulation state");
    } else {
        println!("  PASS: seed 43 differs from seed 42 (snapshot observes state)");
    }
    println!();

    // --- 4. golden ----------------------------------------------------------
    let golden_path = Path::new(GOLDEN);
    if write_golden {
        std::fs::create_dir_all("goldens").expect("create goldens dir");
        std::fs::write(golden_path, reference).expect("write golden");
        println!("wrote {GOLDEN}: {}", Fingerprint::of(reference));
    } else {
        match std::fs::read_to_string(golden_path) {
            Ok(golden) => {
                println!("golden {GOLDEN}: {}", Fingerprint::of(&golden));
                if golden == *reference {
                    println!("  PASS: this build matches the committed golden");
                } else {
                    ok = false;
                    println!("  FAIL: snapshot differs from the committed golden");
                    let stat = bevy_devcards_experiments::diff::diff(&golden, reference);
                    println!("        {stat}");
                    std::fs::create_dir_all("target").ok();
                    let out = std::path::Path::new("target/e41_actual.scn.ron");
                    std::fs::write(out, reference).ok();
                    println!("        actual written to {}", out.display());
                }
            }
            Err(e) => {
                ok = false;
                println!("  FAIL: cannot read {GOLDEN}: {e}");
                println!("        run with --write to create it");
            }
        }
    }
    println!();

    // --- 5. transcendental probe (not part of the golden) -------------------
    // The card sim avoids libm on purpose. This records what would happen if it
    // did not, so the cross-machine risk is measured rather than guessed at.
    println!("transcendental probe (informational, excluded from the golden):");
    for x in [0.5f32, 1.0, 2.0, 123.456] {
        println!(
            "  x={x:<8} sin={:08x} cos={:08x} sqrt={:08x}",
            x.sin().to_bits(),
            x.cos().to_bits(),
            x.sqrt().to_bits(),
        );
    }
    println!("  compare these lines between the local run and the CI run: sqrt is");
    println!("  IEEE-exact and must match; sin/cos are platform libm and may not.");
    println!();

    println!("{}", if ok { "E41 PASS" } else { "E41 FAIL" });
    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
