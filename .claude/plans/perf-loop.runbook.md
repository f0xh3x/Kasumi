# Runbook: performance + CSS clean-up loop

**Pattern:** sequential · **Mode:** safe · **Branch:** `shell`
**Baseline (2026-10-10):** `cargo test --features tauri/custom-protocol` → 22 passed, 0 failed, 2 ignored.

One iteration = one phase. Each phase is its own commit (message explains *why*,
ends with the Co-Authored-By line from CLAUDE.md §1). Do not start the next phase
until the current one has passed every gate.

## Phases (in order)

| # | Phase | Main files | Needs the user |
|---|---|---|---|
| 0 | Baseline numbers + `backup/perf-start` tag + fix "21 tests" → 22 in CLAUDE.md | tools/, CLAUDE.md | mouse, install |
| 1 | Targeted data emit; incremental `taskIcons` | providers/mod.rs, providers/tasks.rs, data.ts, widgets/tasks.ts | check in app |
| 2 | Engine idle back-off for cursor polling | engine.rs | hover capture |
| 3 | `ResizeObserver` instead of 2 s `checkWidths` | main.ts | check in app |
| 4 | Media timer gated on visibility; `mirrorPath` removed; (optional) event-driven battery/network | media-panel.ts, geometry.ts, main.ts, system.rs | check in app |
| 5 | Visuals to CSS (`--v` fills, option sizes, hover looks) | styles.css, widgets/*.ts, main.ts | screenshots |

## Gates (every iteration, in order, all must pass)

1. `npm run build` (tsc + vite) — no errors.
2. `cd src-tauri && cargo test --features tauri/custom-protocol` — still 22 pass.
3. `cargo build --release --features tauri/custom-protocol` — compiles (no install).
4. Review: `rust-reviewer` for Rust changes, `typescript-reviewer` for TS changes;
   then `/simplify`. Fix findings, re-run gates 1–3.
5. Rules check against CLAUDE.md §4: no `will-change`, no mirrored boxes with text,
   no heights animated in fractions, no hook inside the app.
6. Leftover grep (CLAUDE.md rule 7): options, CSS hints and comments for what was removed.
7. Commit on `shell`.

## Pause points (the loop stops and asks the user — never skips them)

- Anything that moves the mouse, clicks or types (baseline, hover capture).
- `tools/install.sh` (it restarts the app the user is working in).
- A visual check that needs the user's eyes (shimmer, jumps).
- Never `taskkill /F`; never toggle Caps Lock.

## Stop conditions (explicit)

Stop and report when ANY of these is true:
- Phase 5 is committed and gates pass (success).
- A gate fails twice in a row on the same phase (revert that phase's work with
  `git restore`/new commit, report why).
- A phase's before/after numbers show no gain (report; ask whether to keep it).
- 6 iterations have run (hard cap).
- A pause point is reached and the user has not answered.

## Progress log

| Phase | Status | Commit | Before → after |
|---|---|---|---|
| 0 | pending | | |
| 1 | pending | | |
| 2 | pending | | |
| 3 | pending | | |
| 4 | pending | | |
| 5 | pending | | |
