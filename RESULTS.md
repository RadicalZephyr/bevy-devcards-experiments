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

# Phase 0 — Baselines

Run after Phases 4 and 1, which was the wrong order: the plan says E01 "bounds
the grid regardless of which approach wins, so it's worth knowing before you
invest in any of them", and Phase 3 was about to be invested in. Both baselines
pass, and together they settle Gate 3.

## E00 — headless world tick cost

**Verdict: ~1000 realistic card worlds tickable per frame.**

`cargo run --release --bin e00_tick_cost`

Measured two ways, because they are not the same question. One world ticked many
times isolates schedule overhead; N worlds ticked once each round-robin is what a
card grid actually does, and touches N separate archetype stores per pass.

| shape | 1 world | 64 worlds, round-robin | cards @60fps | isolation penalty |
| --- | --- | --- | --- | --- |
| empty world, 1 no-op system | 5.3 µs | 8.9 µs | ~1876 | 1.67x |
| 200 entities, 5 systems | 15.5 µs | 17.0 µs | ~980 | 1.09x |
| the Phase 4 card sim | 7.9 µs | 15.6 µs | ~1070 | 1.96x |

Two things worth keeping:

**An empty card still costs ~5 µs to tick**, because it runs the whole `Main`
schedule — `First`, `PreUpdate`, `RunFixedMainLoop`, `Update`, `PostUpdate`,
`Last`, plus the message update system. That is the floor no matter how trivial
the card is, and it caps the design at roughly 1900 cards per frame even if they
do nothing at all.

**Isolation is cheap.** Spreading ticks across 64 separate worlds rather than
repeating them on one costs between 1.1x and 2.0x. Whatever eventually stops
this design, per-world cache behaviour is not it.

A second data point, from the CI smoke run on a GitHub `ubuntu-latest` runner —
shared hardware, so the magnitudes are indicative rather than measurements:

| shape | 64 worlds, round-robin | cards @60fps | penalty |
| --- | --- | --- | --- |
| empty world, 1 no-op system | 2.4 µs | ~6858 | 1.19x |
| 200 entities, 5 systems | 4.4 µs | ~3802 | 1.19x |
| the Phase 4 card sim | 3.8 µs | ~4441 | 1.29x |

About 4x the local laptop figures, with the same ordering and a slightly smaller
isolation penalty. So the honest range for a realistic 200-entity card is
**~1000 worlds per frame on a laptop, ~3800 on a desktop-class machine**. Quote
the laptop number; it is the one that constrains a user.

*Methodology note, because the first run of this got it wrong:* the initial
harness reported the empty world at 2x the cost of the busy one. That was CPU
frequency ramp on the first timed loop of the process, not a finding. The harness
now spins the clock up before timing anything and takes best-of-5 over a fixed
world-tick count.

## E01 — render-to-texture camera cost

**Verdict: 32 thumbnails at 128x128, 16 at 512x512, on integrated graphics.**

`cargo run --release --features render --bin e01_rtt_camera_cost`

N cameras in one world, each targeting its own `Image`, sharing a 24-sprite
scene. Adapter: **Intel HD Graphics 620 (integrated)** — a deliberately weak
baseline, since it is what a lot of users have.

| cameras | 128x128 ms/frame | 512x512 ms/frame |
| --- | --- | --- |
| 1 | 1.53 | 1.79 |
| 2 | 1.78 | 2.01 |
| 4 | 2.95 | 2.91 |
| 8 | 4.65 | 5.73 |
| 16 | 7.14 | 9.70 |
| 32 | 10.98 | **18.08** |
| 64 | **23.35** | **36.64** |

**The cost is per-pass, not per-pixel.** 512x512 is sixteen times the pixels of
128x128 and costs about 1.5x per camera — roughly 0.35-0.6 ms either way, nearly
flat as the count grows. The consequence for the UI is the opposite of the
intuitive one: **make thumbnails bigger rather than more numerous.** Going from
128px to 512px costs about half a camera; adding a camera costs a whole one.

Two caveats on the numbers above. The scene is deliberately small, so these are
ceilings rather than typical. And "in budget" spends the *entire* 16.67 ms on
rendering, leaving nothing for simulation or UI — on a realistic half-budget
split it is closer to **16 thumbnails at 128px and 8 at 512px**.

## Gate 3 — pass

> Fewer than six simultaneous thumbnails in budget means the grid isn't a grid.

Sixteen to thirty-two fit on integrated graphics, and eight to sixteen on a
realistic budget split. The grid is a grid, and "many states at once" survives as
the pitch.

The asymmetry the plan suspected is real and large:

| | in one 16.67 ms frame |
| --- | --- |
| card worlds that can **tick** | ~1000 |
| card worlds that can **draw** | ~16-32 |

That is a factor of roughly 30-60. The design is **"many simulating, few
visible"**, it is not a close call, and the UI should be built around that
asymmetry from the start rather than discovering it later. Simulation is
effectively free; a thumbnail is the scarce resource.

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

Superseded by E00, which measures this properly: ~980 worlds per frame for a
200-entity card, round-robin. E40's number is higher because its cards are
smaller and it ticks them through an `App` rather than directly. Quote E00.

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
`fnv1a64=d92f1bb50c6814be`.

**Every machine and profile produced that same fingerprint**, which is a stronger
result than the plan asked for — it wanted "your machine vs a CI runner", and
this is four environments and two toolchains:

| environment | toolchain | fingerprint |
| --- | --- | --- |
| local Linux x86_64, debug | nightly 1.100.0 | `d92f1bb50c6814be` |
| local Linux x86_64, release | nightly 1.100.0 | `d92f1bb50c6814be` |
| CI ubuntu-latest, debug | stable | `d92f1bb50c6814be` |
| CI ubuntu-latest, release | stable | `d92f1bb50c6814be` |
| CI macos-latest, release | stable | `d92f1bb50c6814be` |
| CI windows-latest, release | stable | `d92f1bb50c6814be` |

Reflection snapshots are the default golden artifact. Pixel diffing stays
opt-in, and the cross-driver screenshot tarpit stays off every user's CI.

One operational caveat, cheap to get wrong: the golden must be marked `-text` in
`.gitattributes`. Without it a Windows checkout rewrites LF to CRLF and E41
fails for a reason that has nothing to do with Bevy.

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

# Phase 1 — World swap

E10 is staged, and stages 1-3b have reached a Gate 1 answer: **the world swap
fails, for one precise and reportable reason.**

`cargo run --features render --bin e10_swap_render`

## E10 — does a swapped world render at all?

It does not, and the cause is three layers deep. Each layer has a different
answer, which is why the first two are worth recording rather than skipping to
the verdict.

### Stage 1-2 — the coupling report (this is E20, arriving early)

The plan puts "find the minimum set of host resources a card world needs" in
Phase 2. It is not a follow-on: the swap cannot be attempted without it, because
the thing being swapped carries the renderer's own resources away with it.

Rather than the plan's "start with nothing, add resources until it works", this
is a set difference between the host world and a bare card world, which produces
the same list without the guessing:

| | count |
| --- | --- |
| resources in a bare card world | 11 |
| resources in the host world after the render stack | 112 |
| in the host and not in the card | 105 |
| …of which are `Messages<T>` event queues | 56 |
| …of which are actual state | **49** |

The 56 event queues are noise — a card world gets its own by adding the same
plugins. The 49 that remain, by crate: `bevy_asset` 14, `bevy_render` 13,
`bevy_input` 6, `bevy_time` 4, `bevy_a11y`/`bevy_camera`/`bevy_core_pipeline`/
`bevy_diagnostic` 2 each, and one each from `bevy_ecs`, `bevy_image`,
`bevy_transform`, `bevy_world_serialization`.

**`AssetServer` and every `Assets<T>` are in that list.** The plan's premise that
"render world, device, and asset storage are shared by construction because it's
the same App" holds for the first two and not the third. Asset storage is a set
of ordinary main-world resources, so a whole-world swap carries it off and the
incoming card arrives with none of it.

### Stage 3 — the naive swap is blocked by visibility

Swapping in a bare card world, with a migration set containing **every resource
the public API can name**, panics on the first tick:

```
resource does not exist: bevy_render::sync_world::PendingSyncEntity
```

Three of the main-world resources the swap must carry are not reachable from
outside `bevy_render`:

| resource | visibility |
| --- | --- |
| `sync_world::PendingSyncEntity` | `pub(crate)` |
| `extract_plugin::ScratchMainWorld` | private |
| `render_asset::CachedExtractRenderAssetSystemState<A>` | private |

So the *naive* form of Phase 1 — bare card worlds handed to a renderer built for
a different world — cannot be built against the public API. That turns out not
to be the real obstacle.

### Stage 3b — building the card world through the same plugin stack

If a card world cannot be *given* the private resources, it can build them
itself: run the same `RenderPlugin` over it with
`RenderCreation::Manual` borrowing the host's device, queue, adapter and
instance, so no second GPU context is created. `ExtractPlugin::build` then
installs `ScratchMainWorld` and `PendingSyncEntity` into the card world directly.

This works, and it is worth noticing where the mechanism comes from: it is Phase
3's E30 (`RenderCreation::Manual` on a borrowed device) used as a *component of
Phase 1*, not as its fallback. Answering E30 in the affirmative is now a
prerequisite for Phase 1 rather than an alternative to it.

Two further things fall out:

- `RenderInstance` is inserted **only into the render world**, while
  `RenderDevice`, `RenderQueue`, `RenderAdapter` and `RenderAdapterInfo` go into
  both. Borrowing the full set means reaching into `RenderApp` as well.
- The migration set collapses to **one resource**: `bevy_time::TimeReceiver`.
  The host's render app holds the matching `TimeSender`; a card with its own
  channel leaves the host's sender writing into a queue nobody drains, and
  `bevy_render::send_time` panics with "The TimeSender channel should always be
  empty during render". Everything else the card builds for itself.

One card, swapped in once, then ticks fine.

### The blocker: `Extract` caches a `SystemState` bound to one `WorldId`

Round-robin across three card worlds panics on the **second** swap, in every
render-world system that uses `Extract`:

```
Encountered a panic in system `bevy_core_pipeline::core_3d::extract_camera_prepass_phase`!
Encountered a mismatched World. This SystemState was created from WorldId(71),
but a method was called using WorldId(106).
```

The cause is `bevy_render::extract_param`:

```rust
fn init_state(world: &mut World) -> Self::State {
    let mut main_world = world.resource_mut::<MainWorld>();
    ExtractState {
        state: SystemState::new(&mut main_world),
        main_world_state: Res::<MainWorld>::init_state(world),
    }
}
```

`Extract<P>` builds a `SystemState<P>` against whichever main world happens to be
installed the first time the extract system initialises, and `SystemState`
validates `WorldId` on every subsequent use. The first swap is survivable because
the states have not been initialised yet — they bind to card 0. The second swap
presents card 1, a different `World`, and every extract system rejects it.

The sequence is exact: swap 1 (host → card 0) ticks; swap 2 (card 0 → card 1)
panics.

## Gate 1 — fail

The gate asks for two worlds rendering, a bounded render-world entity count, and
entity mapping that is clean or fixable by offsetting. The first condition fails
before the other two can be measured, so E11 and E12 are moot **for this
approach**: the render world never sees a second main world at all.

Per the plan, fail → Phase 3.

### The negative result, stated for upstream

This is the artifact the plan says is worth writing carefully, so here it is in
one paragraph.

> In Bevy 0.19, the render world cannot be driven by more than one main world.
> The obstruction is not the retained render world's entity mapping, which is
> where it was expected — it is `bevy_render::extract_param::Extract`, which
> caches a `SystemState` built against the main world present at system
> initialisation and validates `WorldId` on every use. Any main world other than
> that one panics at the first extract. Three supporting main-world resources
> (`PendingSyncEntity`, `ScratchMainWorld`,
> `CachedExtractRenderAssetSystemState<A>`) are also not public, which blocks the
> simpler workaround of migrating them; that is surmountable by building each
> candidate main world through the same `RenderPlugin` with
> `RenderCreation::Manual`, but the `WorldId` binding is not.

Directly relevant to Bevy's own editor effort, and it is the README section
explaining why cards are one at a time rather than an apology for it.

### Does this still matter, now that Gate 2 passes?

Less than it looked at the time, and it is worth being clear about that rather
than leaving a dramatic negative result standing unqualified.

The world swap was one route to isolated card worlds. Phase 3 reaches the same
place by a different one, and because the multi-App design never swaps a main
world, the `WorldId` binding that killed Phase 1 cannot arise there. Phase 1 is
a dead end, not a blocked road.

What survives is the upstream report below. It is still the only precise account
of why one render world cannot serve two main worlds, it is still relevant to
Bevy's editor work, and it is still worth filing.

---

# Phase 3 — Shared device, multiple Apps

The plan treats this as the fallback if Phase 1 fails its gate. It is not a
fallback; it is the design. **Gate 2 passes, using nothing but public API.**

## E30 — does a second App boot on a borrowed device?

**Verdict: yes.** Answered in passing by E10 stage 3b, before Phase 3 was
started. `RenderCreation::manual(device, queue, adapter_info, adapter, instance)`
boots additional Apps in-process with no second GPU context. All five handles are
publicly nameable.

One wrinkle: `RenderInstance` is inserted **only into the render world**, while
`RenderDevice`, `RenderQueue`, `RenderAdapter` and `RenderAdapterInfo` go into
both. Borrowing the full set means reaching into `RenderApp` as well as the main
world.

## E31 — cross-App texture handoff

**Verdict: PASS.** Three separate Apps each rendered into a texture the host
owns, in one process, on one device, with no crosstalk.

`cargo run --release --features render --bin e31_cross_app_texture`

### The handoff goes the other way round

The plan asks: "Can the host's render world be handed a `GpuImage` pointing at
the card's texture?" The answer is to not do that. Reverse it:

1. the **host** creates an ordinary `Image` and lets its own render world build
   the `GpuImage`,
2. that `GpuImage`'s `TextureView` is handed to the card App's
   `ManualTextureViews`,
3. the card's camera targets `RenderTarget::TextureView(handle)`.

The host then samples its own `Image` like any other texture. `RenderAssets`
being per-app stops being a problem, because nothing is ever injected into the
host's asset storage — no fighting the host's render-asset pipeline for ownership
of an `AssetId`, and no lifetime question about a texture owned by another App.

`RenderTarget::TextureView` is documented as the hook for views "created outside
of Bevy, for example OpenXR", so this is a supported path rather than a crack
being squeezed through.

### Result

Each card clears to a distinct pure colour, and the host reads its own textures
back through `Readback`. Control read first, so a pass cannot be a texture that
happened to already contain the right thing.

| texture | before | after | uniform |
| --- | --- | --- | --- |
| 0 | `[0,0,0,255]` | `[255,0,0,255]` | yes |
| 1 | `[0,0,0,255]` | `[0,255,0,255]` | yes |
| 2 | `[0,0,0,255]` | `[0,0,255,255]` | yes |

Distinct colours are the crosstalk check: if card 1's output had landed in card
2's texture, this table would show it.

### How deep did we have to reach?

Not at all. The plan asks this because "works, but requires private wgpu access"
is a different answer from "works, but is ugly", and it decides whether this is a
crate or an upstream PR.

| step | API |
| --- | --- |
| borrow the device | `RenderCreation::manual(..)` |
| read the host's `TextureView` | `RenderAssets<GpuImage>::get(..)` |
| hand it to the card | `ManualTextureViews` (public, `DerefMut` to `HashMap`) |
| point the card's camera at it | `RenderTarget::TextureView(handle)` |

No private items, nothing reached past Bevy into wgpu. **This is a crate.**

## E32 — the duplication tax

**Verdict: the tax is exactly what was feared, and it is avoidable.**

`cargo run --release --features render --bin e32_duplication_tax`

A 2048x2048 RGBA8 texture, 16 MiB a copy — the same order as the plan's 20MB
mesh, and a texture needs no material or pipeline to get uploaded, which keeps
the measurement clean. VRAM read from wgpu's own allocator report rather than
inferred from the bytes requested, so the numbers include real driver padding.

**Four Apps each loading their own copy:**

| | VRAM | delta |
| --- | --- | --- |
| baseline | 0.7 MiB | |
| card 0 | 16.8 MiB | +16.2 |
| card 1 | 33.0 MiB | +16.2 |
| card 2 | 49.2 MiB | +16.2 |
| card 3 | 65.4 MiB | +16.2 |

**4.05x.** Precisely the outcome the plan named as a hard ceiling.

**Four Apps sharing one GPU texture:**

| | delta |
| --- | --- |
| owner App uploads | +16.2 MiB |
| each of 4 borrowers | +0.2 MiB |

**1.02x.** One upload backs all five Apps. The 0.2 MiB per borrower is that
App's own overhead, not a share of the asset.

The mechanism is the same insight as E31: the Apps already share a device, so
they can share what sits behind it. `GpuImage` is cheap to clone and the clone
points at the same `wgpu::Texture`; `RenderAssets<GpuImage>::insert` is public.

### This is an API requirement, not an optimisation

The tax only disappears if **the crate owns asset loading** and hands cards
references, rather than letting each card call its own `AssetServer`. A card that
loads its own assets pays the full 4x, and no amount of later optimisation
recovers it — by then there are four textures on the GPU.

That has to be designed in from the start, and it constrains the card API: a card
cannot be handed a bare `AssetServer` and left to its own devices. It is the
second thing on this list (after the coupling report) that the crate must take
ownership of rather than delegate.

## Gate 2 — pass

> Shared-device multi-App works without reaching past Bevy's public API.

It does. The grid is on the table for v1.

### The architecture that falls out

One App per card, one shared device, and **the host owns every texture**. Cards
render into them; the host composites them like ordinary sprites.

Worth noticing what this sidesteps. Gate 1 failed because `Extract` binds a
`SystemState` to the `WorldId` of whichever main world was installed at
initialisation. The multi-App design **never swaps a main world** — each App
keeps its own permanently, so each App's extract systems bind once and stay
bound. The obstruction that killed Phase 1 cannot arise here.

E11 and E12 die with Phase 1 rather than moving behind E31, for the same reason.
There is no shared retained render world to grow without bound, and no
`MainEntity` keyspace shared between cards: each App has its own render world, so
the 100% entity-id overlap E40 measured is harmless. **Offsetting entity
allocators is not needed.**

The remaining risk was E32, the asset duplication tax, and it is measured above:
real at 4.05x if cards load independently, 1.02x if the crate owns loading. It
constrains the API rather than the architecture.

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

## Finding 3 — Phase 1 and Phase 3 are not alternatives

The plan treats Phase 3 (shared device, multiple Apps) as the fallback if Phase 1
fails its gate. E10 stage 3b shows they overlap: the only way to give a card
world the private resources it needs is to build it through the same
`RenderPlugin` with `RenderCreation::Manual` on the host's borrowed device —
which is E30's mechanism. Phase 1 *contains* Phase 3's first question rather than
being an alternative to it.

Practical consequence: E30 is already answered in the affirmative. A second (and
third) App boots on a borrowed device, in-process, with no second GPU context.
Phase 3 starts from E31.

## Finding 4 — sub-apps are the wrong mechanism for ordered round-robin

Phase 1's design ticks cards round-robin so each gets `fps/N` and the focused
card gets every frame. Sub-apps cannot express that: their tick order is
pointer-hash order over interned labels and changes between runs (E40). Cards
still *work* as sub-apps — nothing in E40 or E41 depends on the order — but the
scheduling policy has to live in an ordered collection the host drives itself.

That is a cheap change, and it is better to know before Phase 1 builds on
`insert_sub_app`.

## Finding 5 — the entity-id collision is already confirmed

E12 does not need to establish that two worlds collide; E40 already did, at 100%
overlap. E12's real job is narrower than the plan assumes: does the render world
actually *mix them up*, and does offsetting each allocator fix it.

---

# Where this leaves the spike

**All three gates are decided. Two pass, and the one that fails does not matter.**

| gate | verdict |
| --- | --- |
| Gate 1 — world swap | **fail** — `Extract` binds a `SystemState` to one `WorldId` |
| Gate 2 — shared-device multi-App | **pass** — and with nothing but public API |
| Gate 3 — is the grid a grid | **pass** — 16-32 thumbnails on integrated graphics |

The premise holds. A card can be a second place to run the code, the worlds are
genuinely isolated, several of them can be on screen at once, and none of it
requires reaching past what Bevy exposes. **This is a crate, not an upstream PR.**

## The architecture

One App per card. One shared device, borrowed via `RenderCreation::manual`. The
host owns every texture; cards render into them through `ManualTextureViews` and
`RenderTarget::TextureView`, and the host composites the results as ordinary
sprites. Simulation and rendering are decoupled, because they have to be:

| | in one 16.67 ms frame |
| --- | --- |
| card worlds that can **tick** | ~1000 (laptop), ~3800 (desktop-class) |
| card worlds that can **draw** | ~16-32 |

"Many simulating, few visible" is not a fallback position, it is the design. And
since E01 found cost is per-pass rather than per-pixel, the few that are visible
should be **large**, not numerous.

Regression testing is settled independently of all of this: `DynamicWorld`
snapshots are byte-identical across machines, profiles and operating systems, so
text goldens are the default artifact and pixel diffing stays opt-in.

## What is left

Every experiment that could have forced a redesign has been run. What remains is
one non-blocking measurement and the writing.

1. **E13 — dormancy.** The only experiment left, and it cannot change the
   architecture. Phase 4 settled the `Time` half: a card advanced by an explicit
   fixed step, never from real time, resumes after any gap with the delta it
   always had. What remains is change ticks and events in a world that has not
   been ticked for a long time. Worth doing before the API is fixed, because a
   card that behaves differently after being backgrounded is worse than a card
   that doesn't render.
2. **File the Gate 1 report upstream.** Written and ready in the Phase 1 section.
   It is the only precise account of why one render world cannot serve two main
   worlds, and it is relevant to Bevy's own editor work.
3. **Write the crate.** The spike has done its job; nothing here should survive
   into it.

**Dead, and why:**

- **E11, E12.** They assumed one retained render world serving several main
  worlds. In the multi-App design each App has its own render world, so there is
  no shared `MainEntity` keyspace and no unbounded growth to measure. The 100%
  entity-id overlap E40 found is harmless, and **offsetting entity allocators is
  not needed.**
- **E20/E21/E22 as a phase.** E10 stage 2 produced the coupling report and stage
  3b showed the useful migration set is not the 49 resources that differ but the
  handful that must be *shared* rather than duplicated. E22's diagnostic question
  survives as a note below.
- **E50.** It was the yardstick in case everything above failed. Everything above
  did not fail.

## Two things the crate must own

Both fell out of experiments rather than design, and both constrain the API
rather than the architecture — which means they have to be decided early.

**Asset loading.** E32: a card that calls its own `AssetServer` pays a full copy
of every asset, 4.05x across four cards. The crate has to own loading and hand
cards references. This cannot be retrofitted as an optimisation.

**The coupling report.** E10 stage 2: the diagnostic the crate gives a user whose
plugin will not start in a card world is a set difference over resources, and it
is the product as much as the grid is. One hard constraint found —
`ComponentInfo::name()` returns a placeholder unless `bevy/debug` is enabled, so
the crate cannot name a missing resource at runtime in a default release build.

## Open questions, with what is now known

- *Does the migration set want to be declared or inferred?* Declared. Inferring
  it mechanically is easy and produces the wrong list — 49 resources that differ,
  when the useful answer was the one that had to be shared. The interesting
  property is sharing, not presence.
- *Do lab cards and regression cards want the same ceremony?* Probably not. A
  component-level change costs 0.26% golden churn and an entity-count change
  costs 25%, so a regression card needs a golden keyed by something stable while
  a lab card needs no golden at all.
- *What is the right UI for "many simulating, few visible"?* The ratio is 30-60x,
  and thumbnail resolution is nearly free while thumbnail count is not. So:
  fewer, larger, live thumbnails over a long roster of simulating-but-unrendered
  cards.
- *E22's diagnostic.* One hard constraint found: `ComponentInfo::name()` returns
  a placeholder unless `bevy/debug` is enabled, so the crate cannot name a
  missing resource at runtime in a default release build.
