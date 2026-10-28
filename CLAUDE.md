# Kasumi: working notes for Claude

How to change this codebase without re-inventing what's already here. **How it
works** (threads, engine, native renderer, config model) is in `ARCHITECTURE.md`;
read it first if the area is new to you. This file is the rules, the catalogue of
shared code, the lessons behind past bugs, and how to build and verify.

The user isn't a coder: they test by using the app and send screen recordings
(on the Desktop). Explain results in plain words. They're often working in other
apps (Premiere, Illustrator) while you work: see "Testing without the user".

Since 0.7.0 the bars are only native (Direct2D, `src-tauri/src/native/`). The
WebView2 bars they were ported from (`src/main.ts`, `src/widgets/*.ts`,
`styles.css`) are in git at `v0.6.7`; comments that say "ported from" a `.ts`
file mean that. The settings window (and the setup and update notes windows) is
still a web page: `settings.html` + `src/settings/`.

---

## 1. Rules

1. **Look before you write.** Check §2 for a helper that already does it (grep
   `win.rs` too: `reg_set_dword`, `broadcast_setting`… exist). If the same few
   lines are needed in a second place, move them to the shared module named in §2
   rather than copying. One source per fact: e.g. the volume comes from the OSD's
   Core Audio watcher, not a second poller.
2. **Minimal.** New extras are optional and **off by default**. Pure black look.
   Everything configurable in `config.toml` *and* exposed in the settings window.
   Shipped defaults (`src-tauri/default-config.toml`) = the user's *Default*
   preset minus its `[[monitors]]` rules.
3. **Rendering rules** (§4) are not optional: every one of them was a visible bug.
4. **Event-driven over polling** where Windows offers notifications. Pollers
   check `providers::placed(app, "<widget>")` (or their option) and idle when
   nothing shows them.
5. **Verify, then claim.** Build, install, check the log (§5). Draw new looks off
   screen with `--native-snapshot` and look at them (§6). Say plainly what you
   could not test (usually anything needing clicks).
6. **Never** `taskkill /F` Kasumi (the graceful quit hands the tray back to
   Explorer and restores the taskbar). Install with `tools/install.sh`.
7. Dead code goes. When a feature is removed, grep for its leftovers: options
   nobody reads, commands nobody invokes, comments describing it.
8. Git: work on branch `native` (releases fast-forward `main` to it); commit
   messages explain *why*, end with `Co-Authored-By: Claude Opus 5.5
   <noreply@anthropic.com>`. Big or risky changes: back up first (§7).

## 2. Shared code: use these

### Native renderer (`src-tauri/src/native/`)

| Where | What |
|---|---|
| `widgets/mod.rs` | Traits `Widget` (keys, data, width, draw, click, tick/wake, act, tooltip…) and `Panel` (size, draw, hover, click, tooltip, pad, pieces for the cascade…); `Ctx` (`gfx`, `look`, `opts`, `h`, `open`; `body()`, `main(px)`, `secondary(px)`, `hover_bg(a)`), `Opts` (`table`, `str`, `num`, `flag`, `surface`, `device`; a panel of another widget's, the clock's calendar, also gets that widget's table), `Act` (`Run`, `call`, `open`, `Launcher`, `then`), `Reply`, `Piece` + `row_width` / `row_draw` (a widget as a row of text, glyphs, gaps), `Roll` (rolling digits), `fit_text`, `Look`, `create` / `create_panel` / `panel_for` / `widget_type` (`"buttons#2"` → `"buttons"`), `glyph::*` |
| `gfx.rs` | `rect`, `rgba`, `white`, `with_alpha`, `hex`, `Font` (`new`, `icons(px)`), `Gfx`: `text_at` (returns the width), `measure`, `text`, `fill_round`, `fill_rect`, `path` (geometry `Cmd`s), `draw_svg`, `picture` + `fill_picture` (JPEG/PNG bytes), `snap` (whole device px), `clip` |
| `widgets/icon.rs` | `decode` (the providers' `rgba_json`), `AppIcon` (`Style::Color/Mono/Accent`) |
| `widgets/shell.rs` | `battery_glyph`, `network_glyph`, `volume_glyph` |
| `widgets/basic.rs` | `format_bytes` (as Explorer) |
| `geometry.rs` | `compute` (the one shape: notches, bar, panels, merging), `Bump`, `Cmd`, `lerp` |
| `tooltip.rs` | `Tooltip::show` (a widget's / panel's hint), `show_over` (covering another app's) |
| `mod.rs` | `post(Msg)`: anything to the native thread; `Spring` (every animation: `configure`, `step`, `settled`, `snap`) |
| `ctxmenu/` | Kasumi's menus: `bar_menu`, `launcher_menu`; `shell::build` (an Explorer view's selection) / `build_item` (one item: an app); layouts in `layout.rs` |

Native widget conventions: options are the `[widgets.<name>]` table; data comes in
`data(key, value)` for the `keys()` it lists; something to do goes out as an `Act`
(a reply, or `act()` after a tick); panels that list things report `tooltip()`
for what's hovered. Everything runs on the native thread: shared state between
surfaces is thread-local (the launcher's `S`, the clock's timer), keyed by surface
label where it must not leak to other bars (`Opts.surface`).

### Backend (`src-tauri/src/`)

| Where | What |
|---|---|
| `providers/mod.rs` | `publish(app, key, json)` (dedupes, sends it to the bars), `publish_entries`, `placed(app, ty)`, `widget_enabled`, `widget_opts`, `opt_str`, `opt_f64`, `http_agent` + `get_json` (5 s timeout), `rgba_json(w, h, rgba)` (icons), `data_url(mime, bytes)`, `SharedConfig` |
| `ui.rs` | The seam to the native thread: `layout`, `state`, `positions`, `event`, `data`… (the engine and providers never post to bars any other way) |
| `win.rs` | `wide(s)` / `wide_to_string(buf)`, `reg_string`, `reg_dword`, `reg_set_dword`, `broadcast_setting`, `system_accent()`, `font_families()`, `user_sid()`, `window_pid`, `window_title`, `class_name`, `process_path(pid)`, `process_name(pid)`, `window_process(h)` (lower-case exe name), `bitmap_bgra(hbitmap)`, `monitors()`, `cursor_pos`, `app_windows`, `fullscreen_monitor`, foreground/focus helpers, keyboard-layout helpers, `short_time_pattern`, `set_clipboard_text`, `send_keys` |
| `providers/tasks.rs` | `exe_icon(path)`, `item_icon(shell item)`: program icons as RGBA; the dock's pins (`dock_pinned`, `dock_toggle`) |
| `providers/system.rs` | `battery()`; `volume_changed(Some((level, muted)))`: fed by `osd/volume.rs` |
| `osd/mod.rs` | `emit(app, kind, event, payload)`: sends any pop-up where its [osd] screen / spot say (adds `notch`); `show(app, kind, level, muted)`, `quiet_volume(duration)` (no OSD while a slider drags) |
| `keyhook.rs` | The keyboard (and, while the scroll pop-up is on, mouse) hook in a helper process: `set_replace_volume`, `set_start_key`, `set_scroll`, `START_HOTKEY`. Any new global input handling goes here, never a hook inside the app (§4) |
| `engine.rs` | `osd_targets(app, kind)` (bars + notch for a pop-up), `monitor_matches(matcher, monitor, index)` ([[monitors]] rule / OSD screen), `launcher_toggle`, command `fe_log` (the settings page's errors) |
| `launcher.rs` | The app index (`apps`), `shell_path(id)`, Settings pages, Start's pins (`start_pins`), `dock_entry` |
| `config.rs` | `widget_enabled`, `widget_placed`, `widget_hides_when_maximized`, `surface_list`, `resolve_surface` |
| `config_edit.rs` | `config_write` / `config_write_many`: comment-preserving, validated |

New commands (for the settings page) must be listed in `main.rs`'s `generate_handler!`.

### Settings page (`src/settings/`)

| Where | What |
|---|---|
| `dom.ts` | `$`, `glyph`, `input`, `select`, `iconButton`, `toggleSwitch` (role/aria), `row(label, help, control)` |
| `schema.ts` | Declarative fields: toggle, slider, number, text, select, color, list, autostart, action, custom. A `path` *function* returning `null` hides a field (dependent options) |
| `editors.ts` | `editors` registry (custom editors; `itemsEditor(widget, separators, key)` edits any list of buttons), `collapsible`, `WIDGETS` catalogue, config migrations (`migrationOps*`, run by `settings/main.ts`) |
| `ctxmenu-editor.ts` | The context menu editor (Explorer's kinds, the bar menu, the launcher menu) |
| `src/quick-tiles.ts`, `src/power-actions.ts` | Catalogues shared with the native code's lists (keep them in step with `native/widgets/quick.rs`, `launcher.rs`) |

## 3. How to…

* **Add a widget:** checklist in `ARCHITECTURE.md` §5.
* **Add a provider:** a thread with a loop gated by `placed()` (or its option);
  `publish` only sends changes. If Windows can notify you, use that (see
  `osd/volume.rs`). Icons: `rgba_json` → `icon::decode`.
* **Add a setting:** an entry in `schema.ts`; a custom editor only if no field
  kind fits, built with `settings/dom.ts` (`editor: "name:arg"` passes `ctx.arg`,
  e.g. `osdPlace:volume`; `loadMonitors()` lists the monitors). Defaults in
  `default-config.toml` (new extras off).
* **Change the config format:** add a migration in `editors.ts` and list it in
  `settings/main.ts` (old configs are migrated when settings opens).
* **Bottom surfaces:** positions are distances from the screen edge; panels mirror
  their parts (the field by the dock, the far end away from it).

## 4. Rendering rules (each was a bug)

| Rule | Why |
|---|---|
| Heights that animate and text rows move in whole device pixels (`gfx.snap`, rounded row depths) | Fractional positions shimmer or flip by 1 px as springs settle |
| Never grow a surface during an animation without the room reserved | The window moves its edge and the content draws at the old spot: the whole dock jumps |
| A panel's content change → it measures again (`size`) before it draws | Otherwise content pokes out of the shape for a frame |
| Keep the inline error catcher first in `settings.html` | It logs a page that fails to load; the page's own reporting dies with it |
| The tray host must stay quiet (no z-order churn) | A noisy host once cost Explorer ~50% CPU |
| No low-level keyboard or mouse hook inside the app: they live in the `--keyhook` helper process | Windows didn't run an in-app hook (or the hooks after it) while our window had the focus: the Windows key opened Start, PowerToys shortcuts died in our windows |
| Our windows never keep the focus without the open launcher (`close_launcher_on_focus_loss` hands it back) | A bar holding focus it can't use swallowed keys |
| Other apps' windows aren't changed (hidden, restyled): cover or leave them | Hiding gets undone by the app, and Kasumi then never learns when it ends |
| A widget's / panel's `wake()` returns a stored moment (set in `tick`), never `Instant::now() + x` | The surface ticks it once `wake()` has passed, asking again then: "now + x" is always ahead, so it never ticked (the clock's timer, carets, media position: `3f7397a`) |

## 5. Build, install, check

```bash
tools/install.sh            # tsc, vite (settings page), cargo release build, swap exe, start, check
tools/install.sh --test     # + cargo tests (#[ignore]d ones read this PC: run them by name
                            #   with -- --ignored --nocapture; they never change anything)
```

After it starts, the log `%APPDATA%\Kasumi\kasumi.log` says `native renderer
started` and `built overlay-<DISPLAY>-<surface>` for each bar. `tools/windows.ps1`
lists the windows (bars size their window to what shows: ~56 px closed). Lines
starting `[kasumi:<window>]` come from the settings page (errors).

Installer: `npx tauri build`; silent test install
`/S /D=%LOCALAPPDATA%\Programs\Kasumi`.

## 6. Testing without the user

* **Ask before** moving the mouse, clicking or typing: they're usually working.
  Never toggle Caps Lock. A test that backs up and restores `config.toml` undoes
  settings they change meanwhile: ask first.
* **Off screen first:** `kasumi.exe --native-snapshot <dir> [--config f.toml]
  [--data values.json]` draws every surface of a config in its states into PNGs
  (the launcher searching, sums, Settings pages, clipboard, pop-ups, panels, the
  scroll bar) without touching the running Kasumi. Put test configs and data in
  the job's tmp, crop with ffmpeg, look at them.
* **Read-only probes:** `#[ignore]` tests that read this PC (scroll positions,
  Start's pins, launcher menus, radios, Bluetooth batteries, tooltips) show what
  Windows hands Kasumi without changing anything.
* **Before / after screenshots:** `tools/snap.ps1 -tag before`, change, install,
  `tools/snap.ps1 -tag after`, then `python -I tools/frames.py compare <dir> before after`.
* **Hover the dock open/closed while capturing ~60 fps** (moves the mouse: ask):
  `tools/hover-capture.ps1 -monitor <n>`, then `frames.py changes` and
  `frames.py track`.
* **Key presses** (ask first): `tools/keys.ps1 -steps "LWin LWin Escape"`;
  `-physical` makes the keyboard helper treat them as real presses. Never Caps
  Lock, Win+D or Alt+F4 combos.
* **Notifications and windows:** `tools/toast.ps1 -title .. -text ..`,
  `tools/testwin.ps1` (a throwaway window that closes itself). Never move or
  maximize the user's own windows for a test.
* **The user's recordings:** `frames.py extract <video> <dir>`, then the same
  analysis. Measure: a 1 px jump is invisible in a zoomed screenshot and obvious
  in numbers.
* **The settings page:** headless Edge with a stubbed `window.__TAURI_INTERNALS__`
  (the user's config, read only) serves `dist/` over `python -m http.server` and
  screenshots it (module scripts don't load from `file://`).

## 7. Release and backups

1. Bump the version in `package.json`, `src-tauri/tauri.conf.json`,
   `src-tauri/Cargo.toml` (line 3).
2. `default-config.toml` = the Default preset minus `[[monitors]]`;
   `python -I tools/ship_presets.py` regenerates the shipped presets (not blindly:
   the user's saved presets lack keys the shipped ones have on purpose).
3. Fast-forward `main` to `native`, tag `vX.Y.Z`, `npx tauri build`.
4. Back up to `C:\Users\Kip\dev\StatusNotch-backups\<version>\`: exe, installer,
   `config.toml`, presets, notes, `git bundle create … --all`. Before a risky
   change: same, plus a `backup/<name>` tag.
5. Publish (only when the user says so): `git push origin main native vX.Y.Z`, then
   `python -I tools/release.py vX.Y.Z <installer> <notes.md>` creates the GitHub
   release with the installer attached (its token comes from Git Credential
   Manager, never printed). The app's updater (updater.rs) finds new versions
   there without GitHub's API (rate-limited): the latest-release redirect, and the
   installer by name, so it must be `Kasumi_<version>_x64-setup.exe` (what `npx
   tauri build` makes). Commits use the GitHub no-reply email (repo-local
   `user.email`), never the user's own address.

## 8. Environment notes

* PATH for builds: `export PATH="/c/Program Files/nodejs:$HOME/.cargo/bin:$PATH"`
  (`tools/install.sh` sets it).
* Python scripts that patch files: write them with the Write tool. Heredocs
  mangle backslashes (`\\`, `\s`, `\N`, `\n` in Rust strings).
* `ffmpeg` is on PATH (WinGet Gyan.FFmpeg).
* The user's monitors (they change): ARZOPA 1920×1080, G27Q 2560×1440 (primary),
  sometimes a Lenovo P27q-20 portrait. Top bars plus bottom docks on each.
* `.scratch/` (git-ignored) is throwaway; anything worth keeping goes in `tools/`.
* `native-test/`: the first native bar and dock prototype, its own Cargo project,
  kept for reference; Kasumi doesn't build or use it.
* Measure CPU with `QueryProcessCycleTime`: process CPU times move in 15.6 ms
  ticks. The shell's imaging stack costs ~13 MB in any process that loads it.
