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
| 0 | done (idle only; hover capture needs the mouse) | 6830497 | idle: 1.31 CPU-s / 20 s; 6 WebView2 221 MB, statusnotch 116 MB |
| 1 | done, reviewed (2 findings fixed) | b1b6a57 | not yet measured (needs install) |
| 2 | done, reviewed (flick latency: band widened to 200 px, 40 ms calm poll, side borders, zone sizes) | 609807f + follow-up | not yet measured (needs install) |
| 3 | **dropped**: a ResizeObserver already exists; the 2 s check is the deliberate fix from 6e2ce6e | | |
| 4 | partly: media timer gated. **Skipped** `mirrorPath` removal (path built by many string emitters in an untested file; regex runs only while a bottom dock animates) and event-driven battery/network (most work, least gain) | 35da19f | |
| 5 | done, reviewed clean; CSS values verified in headless Edge | 85bc7ed | n/a (code clarity, not a measured gain) |

## Measured (2026-10-10, installed and checked on the real machine)

- **Windows:** six overlays 345 / 709 px, no page errors in the log.
- **Hover capture, 60 fps, G27Q dock:** old and new builds open and close in the same
  5 frames each, same x-ranges, same latency (first change at frame 2).
- **Screenshots:** bottom docks pixel-identical on all three monitors; top bars differ only
  by the clock, the scrolling media title and the window behind. Task icon and battery
  data show in the opened dock (taskIcons deltas and system data arrive).
- **Idle CPU, interleaved A/B (old exe then new exe, 2 rounds x 3 runs, cursor parked,
  main process + all WebView2 counted):**
  old 0.83 CPU-s / 20 s (main 0.25, webview2 0.58), new 0.92 (main 0.30, webview2 0.62).
  **No measurable improvement at idle**; the difference is within noise. The whole app is
  ~4 % of one core. Earlier unpaired samples (1.3-2.7) were the user's other apps.
- **Back-off verified with a temporary tick counter:** mid-screen 125 ticks / 5 s (40 ms),
  60 px from the top edge ~306 ticks / 5 s (16 ms). It engages, but a tick is so cheap
  that 60 % fewer of them does not show in CPU: the engine loop was not the cost.
- **Not tested:** widgets moving between monitors / a layout change while running (needs a
  config.toml edit; the rule is to ask first).
- **Measuring trap:** `StatusNotch.old.exe` has process name `StatusNotch.old`; filtering
  on `statusnotch.exe` silently leaves the old main process out of the numbers.

## Still open

1. Widgets moving between monitors / changing a layout while running (Phase 1 `resend`):
   needs a temporary config.toml change; do it with the user's OK.
2. Possible follow-up: six `will-change` declarations remain on `.bar .slot .panel .osd .peek`
   (CLAUDE.md section 4 bans them on content; these are structural, so untouched).
3. If idle cost matters: it is ~0.85 CPU-s / 20 s in total, ~70 % in the six WebView2
   processes. Next suspects are per-second page work (clock text, media scroll), not the
   engine or the data flow. Profile a page (DevTools Performance) before changing anything.
