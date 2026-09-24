# Handoff — bevy-devcards feasibility spike

Written 2026-09-22, skills section amended 2026-09-24. Next session: continuing
the spike on a **different computer**.

## State in one paragraph

The spike is **finished**. All three gates from the plan are decided, and every
experiment that could still change the outcome has been run and pushed. The
answer to the spike's question is yes: N isolated `World`s in one process,
sharing a GPU device, several visible at once, using only Bevy's public API.
What remains is not experimental — it is writing the crate, and filing one
upstream issue that cannot be filed as-is (see *Hard constraints*).

## Read first

- **`RESULTS.md`** in the spike repo. That is the deliverable and it is current.
  Every number, verdict, gate and carry-forward is there. **Do not re-derive any
  of it from this handoff** — this document deliberately does not repeat it.
- `README.md` — what each experiment binary answers and how to run it.
- `git log` — the commit messages carry the reasoning for each finding.

## Where things live

| what | where |
| --- | --- |
| spike repo | `git@github.com:RadicalZephyr/bevy-devcards-experiments.git` (public) |
| the actual crate (early) | `git@github.com:RadicalZephyr/bevy_devcards.git` (public) |
| local paths (old machine) | both under `/storage/zefira/prog/bevy/` |
| the plan the spike executes | Claude artifact `de37767f-a2eb-4875-b640-593dc34a0838` — not in any repo |

Both repos are public on GitHub and clone cleanly onto a new machine.
`bevy_devcards` holds `design/vision.md`, the Clojurescript-Devcards framing that
motivates the whole thing. Verified 2026-09-22 against the server:
`refs/heads/main` = `8d10a4c`, matching local, sole branch, no tags.

**One portability risk remains:** the **plan** the spike executes lives only as a
Claude artifact, `de37767f-a2eb-4875-b640-593dc34a0838`, and is in no repo.
Retrieve it with the Artifact tool (`action: "read"`) if the original wording is
needed; the spike's deviations from it are already recorded in `RESULTS.md`.

**One loose end in `bevy_devcards`:** its local `main` has **no upstream
tracking** configured, so bare `git push` / `git pull` / `git status` there will
not report ahead/behind. One-time fix on whichever machine holds it:

```bash
git branch --set-upstream-to=origin/main main
```

## What "continuing" can mean now

The spike has no remaining experiments. Pick deliberately:

1. **Re-run on the new hardware** (see next section) — cheap, and the only
   numbers that are machine-specific are the two that matter most for UI design.
2. **Write the crate.** `RESULTS.md` ends with two constraints the crate must
   own rather than delegate; they constrain the API and cannot be retrofitted.
3. **File the upstream issue** — blocked on a policy constraint, see below.
4. **New questions the spike did not cover.** Explicitly out of scope
   throughout: wasm, multi-window, and anything about Bevy's own editor.

## Machine-dependent results — what to re-run, and what must not move

This is the main reason a machine change matters.

**Re-run and record (hardware-specific; old numbers were an Intel HD 620
integrated GPU on a laptop):**

- `e01_rtt_camera_cost` — the thumbnail ceiling. Gate 3 passed on weak
  integrated graphics; a discrete GPU should do better, and the *shape* of the
  finding (cost is per-pass, not per-pixel) is worth confirming rather than
  assuming.
- `e00_tick_cost` — tick throughput. `RESULTS.md` already carries two data
  points (laptop and CI runner, ~4x apart); a third makes the range honest.
- `e32_duplication_tax` — needs wgpu's allocator report, which **returns `None`
  on some backends**. It was measured on Vulkan. On a Metal or DX12 machine the
  experiment will say so and stop rather than lie.

**Must NOT change — treat a difference as a real finding:**

- `e41_snapshot_determinism` must still match the committed golden
  (`goldens/e41_card.scn.ron`). Matching across machines *is* the result. If a
  new machine disagrees, that is a genuine regression in the cross-platform
  determinism claim and is worth more attention than anything else in this
  document.
- `e13_dormancy` and `e40_subapp_worlds` assert rather than measure. They should
  pass identically.

All experiments run with `cargo run --release --bin <name>`; the Phase 1/3 ones
additionally need `--features render`.

## Environment the new machine needs

- Rust toolchain. CI uses **stable**; the old machine was nightly. Both work and
  both produce the same E41 golden, which is itself a recorded result.
- **Headless experiments need nothing**: `e00`, `e13`, `e40`, `e41`, `e42`. No
  GPU, no display, no system packages — that is Finding 2 in `RESULTS.md` and it
  is deliberate.
- **`--features render` experiments need a working GPU + Vulkan**: `e10`, `e31`,
  `e32`. The old machine had both a real adapter and lavapipe
  (`/usr/share/vulkan/icd.d/lvp_icd.json`) available as a software fallback.
- First `--features render` build compiles wgpu and naga: **budget 10+ minutes**
  and run it in the background rather than waiting on a tool timeout.

## Gotchas that cost time to discover

Rediscovering these is pure waste:

- **The crate is its own workspace on purpose**, with a committed `Cargo.lock`
  and `bevy = "=0.19.1"` pinned exactly. Do not attach it to a parent workspace —
  feature unification would silently change the enabled feature set and make
  E41's golden meaningless. (`Cargo.toml` line ~11 still refers to "the parent
  `0.19/` workspace"; that directory no longer exists, so the comment is stale
  history. Harmless, but do not go looking for it.)
- **`bevy/debug` is required** for `ComponentInfo::name()` to return real names.
  Without it every type reports the same placeholder string, which silently
  collapses the E10 coupling report to one line.
- **`#![recursion_limit = "256"]`** is needed in any binary touching
  `RenderDevice`; wgpu's type graph overflows auto-trait resolution otherwise.
- **`cargo fmt` — verify with stable**, not nightly:
  `rustup run stable cargo fmt --all --check`. CI's rustfmt job uses stable and
  will disagree with a nightly-formatted tree.
- **Benchmark harness lesson**: the first timed loop in a process runs at a lower
  CPU clock. An early version of E00 reported an empty world as *slower* than a
  busy one because of it. The harness now spins up before timing and takes
  best-of-N over a fixed world-tick count — keep that if you add benchmarks.
- **CI shape**: artifact upload is `continue-on-error` (a 403 from the artifact
  service once turned a fully green run red); `e00` runs as a labelled smoke test
  on ubuntu/release only because a shared runner is the wrong place to benchmark;
  `e13` runs on every matrix entry because it asserts.

## Hard constraints

**Do not file the upstream issue as drafted.** `upstream-issue.md` in the spike
repo is marked `DO NOT FILE AS-IS` in its first line. Bevy's contribution policy
(<https://bevy.org/learn/contribute/policies/ai/>, item 2) forbids AI-generated
prose. The *findings* in that file are verified and reusable — panic text,
`file:line` citations, the visibility table, repro steps, and the two related
issue numbers (bevyengine/bevy#18884 and #24483, both checked against the live
tracker). The prose is not. Taking it upstream means a human rewriting it from
scratch, with disclosure that an AI found the issue. **Do not run
`gh issue create` against `bevyengine/bevy` under any circumstances.**

**Two things the crate must own rather than delegate** — both discovered rather
than designed, both constrain the API, neither can be retrofitted. They are
stated with their evidence at the end of `RESULTS.md`; do not restate them from
memory, read them there.

## Suggested skills

Call these with the Skill tool as the work demands. The spike itself used none —
it was straight engineering — so these are for the phase that follows.

**Check availability first.** Skills come from two places and only one of them
follows you to a new machine:

- *claude.ai account skills* travel automatically. Verified 2026-09-24:
  `park`, `domain-modeling`, `grilling`, `handoff`, `writing-for-agents`,
  `wait-what`, `i-have-adhd`, `docs`, `doc-coauthoring`, `skill-creator`,
  `learn`, `import-memory`, and the file-format ones.
- *local skills* in `~/.claude/skills/` do **not** travel unless that directory
  is synced. On the old machine that set was: `domain-modeling`, `grilling`,
  `handoff`, `i-have-adhd`, `research`, `sodium-frp`, `sodium-frp-workspace`,
  `tdd`, `teach`, `wait-what`, `wayfinder`. Note the overlap — the first five
  exist in both places and are safe either way.

Ranked for the work ahead:

- **`park`** — account-level. Worth running on the **old** machine before
  abandoning it, if that has not happened already. It writes a `STATUS.md`,
  commits and pushes, which is the belt to this document's braces.
- **`domain-modeling`** — account-level, and the highest-value one for the next
  phase. The spike produced an architecture decision with measured evidence
  behind it, and it currently exists only as prose in `RESULTS.md`. Recording it
  as an ADR, and settling the vocabulary (card, host, lab card vs regression
  card), should happen before the crate's API solidifies.
- **`grilling`** — account-level. Before committing to the crate design. The
  spike's conclusion reverses the plan's expected architecture, and the two "must
  own" constraints are load-bearing. Worth stress-testing rather than accepting
  because the measurements were clean.
- **`tdd`** — **local only; will be missing on a fresh machine.** If building the
  crate test-first, either sync `~/.claude/skills/` or proceed without it. Worth
  knowing that `e41`'s snapshot machinery already gives a byte-exact oracle for
  "did this change card behaviour", which is an unusually good thing to test
  against with or without the skill.
- **`writing-for-agents`** — account-level. Only if adding a `CLAUDE.md` or
  `AGENTS.md` to the crate repo.

## Redaction note

Nothing sensitive is included here. Both GitHub repositories referenced are
public. No credentials, tokens, or contact details appear in this document or in
either repo.
