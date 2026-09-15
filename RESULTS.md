# Results

Every number here is valid for exactly one Bevy patch. These are internals
questions and the answers expire on version bumps, so nothing in this file
should be quoted without the version line next to it.

**Bevy 0.19.1.** Not 0.18.x as the plan assumed — see [Finding 1](#finding-1--dynamicscene-does-not-exist-in-bevy-019).

## Environment

| | |
| --- | --- |
| Bevy | `=0.19.1`, `default-features = false`, features `std`, `bevy_world_serialization`, `serialize` |
| Local toolchain | `rustc 1.100.0-nightly (a69a63265 2026-09-03)`, x86_64-unknown-linux-gnu |
| Local machine | Linux 7.0.11-76070011-generic (Pop!\_OS) |
| CI | GitHub Actions, `dtolnay/rust-toolchain@stable`, ubuntu/macOS/Windows |
| GPU | not used — Phase 4 never initialises the renderer |

Deliberately disabled, because both would let entropy into the results:

- `multi_threaded` — the schedule executor is single-threaded, so system order
  is the schedule's topological order and nothing else.
- `reflect_auto_register` — the type registry is built by hand in
  `card::register_card_types`, so a dependency cannot silently add a type to the
  snapshot.

---

# Phase 4 — Headless and CI

Run in parallel with Phase 1, per the plan, because it de-risks the half of the
product that does not depend on the render question. All three experiments pass.

## E40 — N worlds as SubApps

**Verdict: PASS.** Card worlds work as headless sub-apps with independent
fixed-step clocks. The simulation half of the product does not depend on
anything Phase 1 is still deciding.

`cargo run --bin e40_subapp_worlds` — 8 cards, 600 host frames.

Every card ticked exactly once per host frame and advanced its own
`Time<Virtual>` by its own step, never the host's:

| card | ticks | particles | virtual elapsed | step |
| --- | --- | --- | --- | --- |
| card0 | 600 | 40 | 9.3750 s | 15.625 ms |
| card1 | 600 | 40 | 18.7500 s | 31.250 ms |
| card2 | 600 | 40 | 28.1250 s | 46.875 ms |
| card3 | 600 | 40 | 9.3750 s | 15.625 ms |

Three findings, two of them unasked-for:

**Sub-app tick order is unstable between runs of the same binary.** This is
stronger than the plan would have guessed. `SubApps::update` iterates
`HashMap<InternedAppLabel, SubApp>`, and `InternedAppLabel` is
`Interned<dyn AppLabel>`, which hashes by *pointer*. Interning is
process-global, so the order is fixed within one run — but it changes between
runs, because the interner's allocations land at different addresses. Three
consecutive release runs, labels inserted `Card(0)..Card(7)` every time:

```
Card(3), Card(5), Card(6), Card(2), Card(4), Card(7), Card(0), Card(1)
Card(3), Card(1), Card(0), Card(5), Card(6), Card(2), Card(7), Card(4)
Card(0), Card(1), Card(6), Card(2), Card(5), Card(4), Card(7), Card(3)
```

E40 records its order to `target/e40_visit_order.txt` and compares against the
previous run, so CI runs it twice and the evidence is in the log.

Harmless for cards that are genuinely isolated — which is the whole premise —
but it has two consequences. A card that depends on running before or after
another is nondeterministic, not merely unspecified. And sub-apps cannot be the
mechanism if the round-robin scheduling Phase 1 describes ("every card ticks at
`fps/N`, the focused card gets every frame") ever needs a *defined* order; that
needs an explicit ordered collection driven by hand, which is what E41's
interleaving test does.

**Entity ids collide across worlds, totally.** 40 of card0's 40 live entities
share a raw `Entity` bit pattern with a live entity in card1. Every freshly
created `World` allocates from index 0, so `MainEntity(Entity)` is not a unique
key across cards. This is E12's hazard, visible for free and confirmed before
Phase 1 starts: the render world's main-to-render mapping will alias unless each
card's allocator is offset.

**Throughput, single-threaded, 200-entity cards:**

| profile | ms per card-tick | cards tickable in 16.67 ms |
| --- | --- | --- |
| debug | 0.4207 | ~40 |
| release | 0.0071 | **~2346** |

Quote the release number. Debug is 59x slower and would put Gate 3 in the wrong
place entirely.

This is a partial early answer to Gate 3 and it points the same way the plan
suspected: if E01 caps visible thumbnails in the single digits while thousands
of worlds can tick, the design is "many simulating, few visible" and the UI
should be built around that asymmetry from the start. E00 and E01 still need
running for the real gate; this is the simulation half only, on a deliberately
small card.

## E41 — snapshot determinism

**Verdict: PASS.** Reflection snapshots are byte-identical across repeated runs,
across profiles, and across machines. Text goldens are viable as the default
regression artifact, which is what dodges the cross-driver screenshot tarpit.

`cargo run --bin e41_snapshot_determinism` — card world at frame 600, serialized
to RON via `DynamicWorld`.

| check | debug | release |
| --- | --- | --- |
| 10 runs, same process, world rebuilt from scratch each time | identical | identical |
| card run alongside two other cards vs. run alone | identical | identical |
| different seed produces a different snapshot | yes | yes |
| matches committed golden | yes | yes |

Golden: `goldens/e41_card.scn.ron`, 12180 bytes, 379 lines,
`fnv1a64=d92f1bb50c6814be` — **the same fingerprint in debug and release.**

The "different seed" row is there to stop the other three passing for the wrong
reason: a snapshot that was accidentally constant would satisfy every identity
check in the table.

The second row is the one worth keeping. A card's snapshot does not depend on
what else is running in the process — which is the property the whole crate
rests on, and it now has a test rather than an assumption.

**Why it is deterministic** (so the result survives a Bevy upgrade being
diagnosed rather than just re-run): `DynamicWorldBuilder` accumulates into
`BTreeMap<Entity, _>` and `BTreeMap<ComponentId, _>`, so entity and resource
ordering is sorted, not hash order. Components within an entity follow archetype
order. Nothing in the path iterates a `HashMap`.

**Transcendental probe.** The card simulation deliberately restricts itself to
IEEE-exact arithmetic, so the golden does not silently depend on platform libm.
The probe records what would happen if it did not:

```
x=0.5      sin=3ef57744 cos=3f60a940 sqrt=3f3504f3
x=1        sin=3f576aa4 cos=3f0a5140 sqrt=3f800000
x=2        sin=3f68c7b7 cos=bed51133 sqrt=3fb504f3
x=123.456  sin=bf4dcee4 cos=bf183f1b sqrt=4131c6f7
```

Carry forward: a card that calls `sin`/`cos` is a different determinism question
from a card that does not, and the crate should say so rather than let users
discover it on a Windows runner.

## E42 — snapshot reviewability

**Verdict: measured, and the answer splits.** A whole-world text golden is
perfectly reviewable for component-level changes and useless for anything that
shifts entity allocation.

`cargo run --bin e42_snapshot_churn` — 379-line baseline.

| change | diff | churn |
| --- | --- | --- |
| A. add a component to one existing entity | +1 −0 | **0.26%** |
| B. change one component value on one entity | +1 −1 | **0.53%** |
| C. spawn one extra entity before the sim starts | +47 −47 | **24.80%** |

A is a literal one-line diff, which is as good as this could possibly be:

```diff
         "bevy_devcards_experiments::card::Position": ((3.8221445, 14.276719)),
+        "bevy_devcards_experiments::card::Selected": (),
         "bevy_devcards_experiments::card::Velocity": ((-0.99176216, 8.550276)),
```

C is not a big diff so much as a *lying* diff. Entities are keyed by their raw
packed `Entity` bits, so one extra startup entity shifts every subsequent id by
one and the diff re-attributes each entity's data to its neighbour's key:

```diff
     4294967117: (
       components: {
         "…::Age": (
-          ticks: 97,
-        ),
-        "…::Position": ((32.57399, 30.91794)),
```

Nothing is actually wrong in the new file. It just cannot be read.

**This is an API finding, not a formatting one.** The fix is not prettier
serialization, it is a stable key: the golden should be keyed by something the
card author controls (a `Name`, a declared id) rather than by allocation order.
Note also that `DynamicWorld` extracts exactly what the type registry knows
about — so "declare a subset", which the plan raises as an open API question,
turns out to be a registry question rather than a new mechanism.

---

# Findings that change the plan

## Finding 1 — `DynamicScene` does not exist in Bevy 0.19

The plan's E41 is written against `DynamicScene`. In 0.19 `bevy_scene` was
rewritten around BSN (Bevy Scene Notation) for composable authoring, and it no
longer serializes worlds at all — the crate has no `serde` or `ron` dependency.

The replacement is a new crate, `bevy_world_serialization`, re-exported as
`bevy::world_serialization` behind the `bevy_world_serialization` + `serialize`
features. `DynamicWorld::from_world` / `DynamicWorld::serialize` is a direct
substitute for `DynamicScene::from_world` / `DynamicScene::serialize`, and the
whole adapter is ~15 lines in `src/snapshot.rs`.

Consequence for the crate: the snapshot half has to target a crate that shipped
in 0.19 and has a much shorter history than `DynamicScene` did. Worth pinning
tightly and re-checking on every Bevy release.

## Finding 2 — Phase 4 needs no system dependencies at all

With `default-features = false` and only `std` + `bevy_world_serialization` +
`serialize`, the headless half builds with no `bevy_render`, no `winit`, no
audio, and therefore no apt packages, no display server, and no GPU. The CI
workflow has no setup step beyond a toolchain.

This matters for the product pitch, not just for this repo: card *simulation*
and snapshot regression can ship to users whose CI is a bare container, whatever
Phase 1 decides about rendering.

## Finding 3 — sub-apps are the wrong mechanism for ordered round-robin

Phase 1's design ticks cards round-robin so each gets `fps/N` and the focused
card gets every frame. Sub-apps cannot express that: their tick order is
pointer-hash order over interned labels and changes between runs (E40). Cards
still *work* as sub-apps — nothing in E40 or E41 depends on the order — but the
scheduling policy has to live in an ordered collection the host drives itself.

That is a cheap change, and it is better to know before Phase 1 builds on
`insert_sub_app`.

## Finding 4 — the entity-id collision is already confirmed

E12 does not need to establish that two worlds collide; E40 already did, at 100%
overlap. E12's real job is narrower than the plan assumes: does the render world
actually *mix them up*, and does offsetting each allocator fix it.

---

# Carried forward into Phase 1

- Offsetting entity allocators (E12) is now a known requirement rather than a
  hypothesis. Budget for it.
- Per-card `Time<Virtual>` advanced by an explicit step, never from real time,
  already gives the dormancy property E13 asks about: a card skipped for N host
  frames resumes with the delta it always had. E13 only needs to check change
  ticks and events.
- `DynamicWorld` snapshots give Phase 1 a cheap oracle: "did swapping worlds
  change the simulation?" is answerable byte-for-byte without rendering.
