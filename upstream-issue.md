# DRAFT — upstream issue for bevyengine/bevy

Not filed. Review before posting.

Suggested labels: `A-Rendering`, `A-ECS`, `C-Bug` or `C-Feature` (maintainer's call — see
*Is this a bug?* below).

---

**Title:** `Extract` binds its `SystemState` to one `WorldId`, so a render world cannot serve more than one main world

---

## Summary

`bevy_render::extract_param::Extract<P>` builds a `SystemState<P>` against whichever
world is in `MainWorld` the first time the extract system initialises, and
`SystemState` validates `WorldId` on every subsequent use. If the main world is
later replaced with a different `World`, every extract system panics.

This is the concrete obstruction behind "one render world can only serve one main
world". I could not find it written down anywhere, and #18884 is `S-Needs-Design`
without mentioning it, so this is offered as a data point for that discussion.

**This is not blocking me** — I found it while spiking a multi-world tool and ended
up solving the problem a different way (one `App` per world on a shared device via
`RenderCreation::Manual`, which works fine and needs nothing private). I am
reporting it because the failure is precise, reproducible, and currently invisible
until you hit it.

## Repro

With Bevy 0.19.1:

1. Build an `App` with `DefaultPlugins` (no window, `ExitCondition::DontExit`).
2. Build two more `App`s with `RenderPlugin { render_creation: RenderCreation::manual(..) }`
   borrowing the first App's device, queue, adapter, adapter info and instance. Take
   each one's main `World`.
3. `core::mem::swap` one of those worlds into the host App's main world slot, then
   `app.update()`. This works.
4. Swap the *other* world in and `app.update()` again.

Step 4 panics:

```
Encountered a panic in system `bevy_core_pipeline::core_3d::extract_camera_prepass_phase`!
thread 'main' panicked at bevy_ecs-0.19.1/src/system/function_system.rs:380:
Encountered a mismatched World. This SystemState was created from WorldId(71),
but a method was called using WorldId(106).
```

Step 3 survives only because the extract systems have not initialised yet — they bind
to the first world swapped in. The second swap is where it fails, and it fails in
every extract system, not just this one.

Runnable version: [`e10_swap_render`](https://github.com/RadicalZephyr/bevy-devcards-experiments/blob/main/src/bin/e10_swap_render.rs),
stage 3b (`cargo run --release --features render --bin e10_swap_render`).

## Root cause

`crates/bevy_render/src/extract_param.rs`:

```rust
fn init_state(world: &mut World) -> Self::State {
    let mut main_world = world.resource_mut::<MainWorld>();
    ExtractState {
        state: SystemState::new(&mut main_world),   // bound to this world's WorldId
        main_world_state: Res::<MainWorld>::init_state(world),
    }
}
```

and on every use:

```rust
let item = state.state.get(main_world.into_inner())?;
```

`SystemState::get` calls `self.validate_world(world.id())`, which panics on mismatch.

So the binding is established once, implicitly, by whichever world happened to be
present at initialisation. Nothing in the type system or the docs suggests the render
world is pinned to one particular main world, but it is.

## Secondary obstruction: three non-public main-world resources

Separate from the above, and worth recording since it blocks the obvious workaround
of preparing a main world by hand. A world handed to the renderer needs these, and
none of them can be named from outside `bevy_render`:

| resource | visibility |
| --- | --- |
| `sync_world::PendingSyncEntity` | `pub(crate)` |
| `extract_plugin::ScratchMainWorld` | private |
| `render_asset::CachedExtractRenderAssetSystemState<A>` | private |

`ScratchMainWorld` is the sharpest case, because `extract` unwraps it:

```rust
let scratch_world = main_world.remove_resource::<ScratchMainWorld>().unwrap();
let inserted_world = core::mem::replace(main_world, scratch_world.0);
```

There is an irony here worth pointing at: `extract` *already* swaps the main world —
that is how it lends the main world to the render world for the duration of
`ExtractSchedule`. The machinery to move a main world in and out exists; it just
assumes it is the only thing doing so.

(Building each candidate world through the same `RenderPlugin` sidesteps this, since
`ExtractPlugin::build` installs them. It does not sidestep the `WorldId` binding.)

## Is this a bug?

Genuinely unclear, which is why I have not picked a label. Bevy has never promised
that a render world can serve several main worlds, so nothing is violating a
documented contract. But the failure mode is a panic deep inside a third-party
system with a message that does not suggest the cause, and it appears one swap later
than the action that caused it. If multiple main worlds are intended to stay
unsupported, a `debug_assert` or a doc note on `Extract` would save the next person
the afternoon it cost me.

## Possible directions

Deliberately not prescribing — this touches design that is already under discussion.

- Re-initialise `ExtractState::state` when `MainWorld`'s `WorldId` changes, rather
  than binding once.
- Key the cached `SystemState` by `WorldId` instead of assuming one.
- Decide explicitly that one render world serves one main world, and say so in
  `Extract`'s docs plus an assertion with a message that names the real problem.

Whichever way it goes, #24483 is refactoring this code right now, so it seems like
the moment to decide deliberately rather than by accident.

## Related

- #18884 — First-class multi-world support. This is a concrete instance of the
  limitations that issue lists; it is not currently mentioned there.
- #24483 — `bevy_extract` tracking. That work generalises extraction to multiple
  *sub-apps* (one main world, several consumers). This report is the mirror image:
  several main worlds, one consumer. Worth knowing about while the code is open.

## Environment

- Bevy 0.19.1
- `rustc` 1.100.0-nightly, `x86_64-unknown-linux-gnu`
- Reproduced headless, no window; not GPU-dependent (the panic is in ECS, before
  anything is submitted)
