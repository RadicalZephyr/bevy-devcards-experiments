# bevy-devcards: feasibility spike

Throwaway experiments answering one question before any crate gets designed:

> Can we run N isolated `World`s in one process, share the GPU device and asset
> storage between them, and get at least some of them onto the screen at once?

Nothing here is a foundation. The deliverable is [RESULTS.md](RESULTS.md) and a
decision, not code anybody keeps.

Bevy is pinned to an exact patch (`=0.19.1`) and this crate carries its own
`Cargo.lock`, because these are internals questions and the answers expire on
version bumps.

## Running

Each experiment is one binary.

```bash
cargo run --bin e40_subapp_worlds
```

| Binary | Phase | Question |
| --- | --- | --- |
| `e00_tick_cost` | 0 | How many card worlds can tick in one frame? |
| `e01_rtt_camera_cost` | 0 | How many render-to-texture cameras fit in one frame? |
| `e40_subapp_worlds` | 4 | Do card worlds work as headless sub-apps with their own fixed-step clocks? |
| `e41_snapshot_determinism` | 4 | Are reflection snapshots byte-identical across runs, profiles, and machines? |
| `e42_snapshot_churn` | 4 | Is a whole-world text golden reviewable in a pull request? |
| `e10_swap_render` | 1 | Does a card world swapped into the App's main world slot render? |
| `e31_cross_app_texture` | 3 | Can a second App render into a texture the host owns? |

Phase 1 needs the renderer, so it sits behind a feature:

```bash
cargo run --release --features render --bin e10_swap_render
```

The benchmarks (`e00`, `e01`) are only meaningful in release.

E41 compares against a committed golden. To regenerate it after an intentional
change to the card simulation:

```bash
cargo run --bin e41_snapshot_determinism -- --write
```

## CI

`.github/workflows/phase4.yml` runs all three on ubuntu (debug and release),
macOS, and Windows. Phase 4 never touches the renderer, so it needs no GPU, no
display server, and no apt packages.

## Layout

- `src/card.rs` — the deterministic card simulation every experiment runs
- `src/snapshot.rs` — reflection snapshot of a world (see the 0.19 note inside)
- `src/diff.rs` — line diff, so churn is a number
- `goldens/` — committed snapshots E41 checks against
