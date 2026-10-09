# StatusNotch: working notes for Claude

How to change this codebase without re-inventing what's already here. **How it
works** (windows, engine, events, config model) is in `ARCHITECTURE.md`; read it
first if the area is new to you. This file is the rules, the catalogue of shared
code, the lessons behind past bugs, and how to build and verify.

The user isn't a coder: they test by using the app and send screen recordings
(on the Desktop). Explain results in plain words. They're often working in other
apps (Premiere, Illustrator) while you work: see "Testing without the user".

---

## 1. Rules

1. **Look before you write.** Check §2 for a helper that already does it. If the
   same few lines are needed in a second place, move them to the shared module
   named in §2 rather than copying. One source per fact: e.g. the volume comes
   from the OSD's Core Audio watcher, not a second poller.
2. **Minimal.** New extras are optional and **off by default**. Pure black look.
   Everything configurable in `config.toml` *and* exposed in the settings window.
   Shipped defaults (`src-tauri/default-config.toml`) = the user's *Default*
   preset minus its `[[monitors]]` rules.
3. **Rendering rules** (§4) are not optional: every one of them was a visible bug.
4. **Event-driven over polling** where Windows offers notifications. Pollers
   check `providers::placed(app, "<widget>")` and idle when the widget isn't on
   any bar.
5. **Verify, then claim.** Build, install, check the window sizes and the log
   (§5). Measure screenshots/frames instead of eyeballing (§6). Say plainly what
   you could not test (usually anything needing clicks).
6. **Never** `taskkill /F` StatusNotch (the graceful quit hands the tray back to
   Explorer and restores the taskbar). Install with `tools/install.sh`.
7. Dead code goes. When a feature is removed, grep for its leftovers: options
   nobody reads, CSS hints (`will-change`), comments describing it.
8. Git: work on branch `shell`; commit messages explain *why*, end with
   `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Big or risky
   changes: back up first (§7).

## 2. Shared code: use these

### Frontend (`src/`)

| Where | What |
|---|---|
| `widgets/index.ts` | `el(cls, ...kids)` (div), `icon(glyph)` (span.icon), `g(code)` glyph char, `Glyph.*` common glyphs, `iconUrl({w,h,rgba})` Rust icon → cached PNG data URL, `createWidget` / `createPanel` / `panelFor` / `widgetType` (`"buttons#2"` → `"buttons"`), types `Widget`, `PanelContext` |
| `data.ts` | `subscribe(key, cb)` → unsubscribe (call it in `dispose`). Typed `DataMap`: system, tasks, taskIcons, temps, media, weather, tray, keyboard, focus, netspeed, recycle, privacy, claude, notifications, clipboard |
| `geometry.ts` | `computeFrame` (the one SVG path), `lerp`, `mirrorPath` (bottom surfaces' clip path; M/L/C/Z only) |
| `spring.ts` | `Spring`: every animation |
| `quick-tiles.ts` | `QUICK_TILES`, `MAX_TILES` (6), `DEFAULT_TILES` |
| `widgets/quick.ts` | `slider()` (icon + range + value; volume, brightness, per-app rows), `slide()` (animate an element's height inside a panel, whole pixels) |
| `settings/dom.ts` | `$`, `glyph`, `input`, `select`, `iconButton`, `toggleSwitch` (role/aria), `row(label, help, control)` |
| `settings/schema.ts` | Declarative fields: toggle, slider, number, text, select, color, list, autostart, action, custom. A `path` *function* returning `null` hides a field (dependent options, e.g. per-app volume hides without the volume slider) |
| `settings/editors.ts` | `editors` registry (custom editors), `collapsible`, `WIDGETS` catalogue, config migrations (`migrationOps*`, run by `settings/main.ts`) |
| `main.ts` | `Overlay`; constants `STAGE_HEIGHT`, `BOTTOM_PANEL_ROOM`, `NOTCH_PAD`; `report(msg)` → app log |

Widget conventions: root `el("widget <type>", …)`; options are the
`[widgets.<name>]` table (+ `_device`: the monitor's device name); interactive
parts get `.clickable`; a panel calls `ctx.resize()` whenever its content changes
size and `ctx.close()` after an action; `panelTrigger: "event"` + a bubbling
`notch:panel` event for panels that open on something other than a click.
Tooltips are plain `title`s (stripped globally when `[general] tooltips = false`).

### Rust (`src-tauri/src/`)

| Where | What |
|---|---|
| `providers/mod.rs` | `publish(app, key, json)` (dedupes, broadcasts `notch://data`), `placed(app, ty)`, `widget_enabled`, `widget_opts`, `opt_str`, `opt_f64`, `http_agent` + `get_json` (5 s timeout), `rgba_json(w, h, rgba)` (icon for `iconUrl`), `SharedConfig` |
| `win.rs` | `wide(s)` / `wide_to_string(buf)`, `window_pid`, `window_title`, `class_name`, `process_path(pid)`, `process_name(pid)`, `window_process(h)` (lower-case exe name), `bitmap_bgra(hbitmap)`, `monitors()`, `cursor_pos`, `app_windows`, `fullscreen_monitor`, foreground/focus helpers, keyboard-layout helpers, `short_time_pattern` |
| `providers/tasks.rs` | `exe_icon(path)`, `item_icon(shell item)`: program icons as RGBA |
| `providers/system.rs` | `volume_changed(Some((level, muted)))`: fed by `osd/volume.rs` |
| `osd/mod.rs` | `show(app, kind, level, muted)`, `quiet_volume(duration)` (no OSD while a slider drags) |
| `keyhook.rs` | The keyboard hook (helper process): `set_replace_volume`, `set_start_key`, `START_HOTKEY`. Any new global key handling goes here, never a hook inside the app (§4) |
| `win.rs` `send_keys` | Simulated key presses (`KeyEvent`, with a mark so a hook can recognise its own) |
| `engine.rs` | `to_screen`, `osd_target`, `overlay_for`, `launcher_toggle`, command `fe_log` |
| `config.rs` | `widget_enabled`, `widget_placed`, `widget_hides_when_maximized`, `surface_list`, `resolve_surface` |
| `config_edit.rs` | `config_write` / `config_write_many`: comment-preserving, validated |

New commands must be listed in `main.rs`'s `generate_handler!`.

## 3. How to…

* **Add a widget:** checklist in `ARCHITECTURE.md` §5. Subscribe in the factory,
  unsubscribe in `dispose`. A panel's growing content: animate it with the
  `slide()` pattern and call `ctx.resize()`; the window already has room
  (`measureBarDrops`).
* **Add a provider:** a thread with a loop gated by `placed()`; `publish` only
  sends changes. If Windows can notify you, use that (see `osd/volume.rs`).
  Icons: `rgba_json` → `iconUrl`.
* **Add a setting:** an entry in `schema.ts`; a custom editor only if no field
  kind fits, built with `settings/dom.ts`. Defaults in `default-config.toml`.
* **Change the config format:** add a migration in `editors.ts` and list it in
  `settings/main.ts` (old configs are migrated when settings opens).
* **Bottom surfaces:** positions are distances from the screen edge; `render()`
  multiplies by `away` (−1 at the bottom) and CSS anchors boxes with `bottom: 0`.
  Panel padding toward / away from the bar: `--pad-near` / `--pad-far`. Order
  inside a panel that should flip: `html.edge-bottom` rules (launcher, quick).

## 4. Rendering rules (each was a bug)

| Rule | Why (commit) |
|---|---|
| Never mirror (`scale: -1`) a box with text in it. Bottom surfaces are laid out from the bottom; only the SVG shape is flipped | A mirrored box redraws its text at sub-pixel offsets whenever its height changes: shimmer (`db021e3`) |
| Heights that animate inside a panel move in whole pixels (`slide()`) | Fractional heights shimmer the rest of the panel (`1f8b087`) |
| A row content is centred in gets a whole-pixel height (`Math.round(bf.rowDepth)`) | The spring's last wobble flipped a half-pixel-centred item by 1 px (`255402b`) |
| No `will-change` on widgets or other content | Own compositing layers get re-rasterized after animations: a 1 px hop 100 ms after the dock opened (`2f28391`) |
| Never grow a bottom window during an animation: reserve room up front (`BOTTOM_PANEL_ROOM`, all dock panels measured) | Windows moves the top edge and the page draws at the old spot: the whole dock jumps |
| Helpers used while modules load are `function` declarations, not `const` arrows | Circular imports (widgets ↔ `widgets/index.ts`): a `const` didn't exist yet and the page died silently (`a5cc490`) |
| Keep the inline error catcher first in `index.html` / `settings.html` | It logs a page that fails to load; the page's own reporting dies with it |
| A panel's content change → `ctx.resize()` (redraws the shape synchronously) | Otherwise content pokes out of the shape for a frame |
| The tray host must stay quiet (no z-order churn) | A noisy host once cost Explorer ~50% CPU |
| No low-level keyboard hook inside the app: it lives in the `--keyhook` helper process | Windows didn't run an in-app hook (or the hooks after it) while our window had the focus: the Windows key opened Start, PowerToys shortcuts died in our windows |
| Our windows never keep the focus without the open launcher (`close_launcher_on_focus_loss` hands it back) | A bar holding focus it can't use swallowed keys |

## 5. Build, install, check

```bash
tools/install.sh            # tsc, vite, cargo release build, swap exe, start, check
tools/install.sh --test     # + cargo tests (21; 2 hardware tests are #[ignore]d:
                            #   cargo test audio -- --ignored, notifications needs WPN_TEST_DB)
```

After it starts: `tools/windows.ps1` lists the windows. Bars ~345 px tall,
docks ~709. **78 / 93 px means the page script never ran**: nothing is drawn.
The log is `%APPDATA%\StatusNotch\statusnotch.log`; lines starting
`[statusnotch:<window>]` come from the pages (errors, `report()`).

Installer: `npx tauri build`; silent test install
`/S /D=%LOCALAPPDATA%\Programs\StatusNotch`.

## 6. Testing without the user

* **Ask before** moving the mouse, clicking or typing: they're usually working.
  Never toggle Caps Lock. A test that backs up and restores `config.toml` undoes
  settings they change meanwhile: ask first.
* **Before / after screenshots:** `tools/snap.ps1 -tag before`, change, install,
  `tools/snap.ps1 -tag after`, then `python -I tools/frames.py compare <dir> before after`.
  Windows behind the bars change too (the user works); look at the crops before
  concluding.
* **Hover the dock open/closed while capturing ~60 fps** (moves the mouse: ask):
  `tools/hover-capture.ps1 -monitor <n>`, then `frames.py changes` (what moved,
  when) and `frames.py track` (sub-pixel position of one item). Some glitches
  only show without a screen recorder running.
* **Key presses** (ask first): `tools/keys.ps1 -steps "LWin LWin Escape"` sends
  combos and reports the window in front after each, flagging the Start menu.
  `-physical` makes the keyboard helper treat them as real presses (it ignores
  simulated Win presses otherwise). Leftover state from a failed run (an
  invisible Search window holding focus) poisons the next one: check `before:`.
  Never Caps Lock, Win+D or Alt+F4 combos.
* **The user's recordings:** `frames.py extract <video> <dir>`, then the same
  analysis. Measure: a 1 px jump is invisible in a zoomed screenshot and obvious
  in numbers.
* **Rendering experiments:** headless Edge is the same engine as WebView2. Build
  a minimal page and render variants:
  `msedge --headless=new --disable-gpu --hide-scrollbars --force-device-scale-factor=1 --window-size=W,H --screenshot=out.png "file:///…#params"`
  (`--dump-dom` + a stubbed `window.__TAURI_INTERNALS__` to test page scripts).
  Prove a cause this way before changing the app.

## 7. Release and backups

1. Bump the version in `package.json`, `src-tauri/tauri.conf.json`,
   `src-tauri/Cargo.toml` (line 3).
2. `default-config.toml` = the Default preset minus `[[monitors]]`;
   `python -I tools/ship_presets.py` regenerates the shipped presets.
3. Merge `shell` → `main` (fast-forward), tag `vX.Y.Z`, `npx tauri build`.
4. Back up to `C:\Users\Kip\dev\StatusNotch-backups\<version>\`: exe, installer,
   `config.toml`, presets, `git bundle create … --all`. Before a risky change:
   same, plus a `backup/<name>` tag.

## 8. Environment notes

* PATH for builds: `export PATH="/c/Program Files/nodejs:$HOME/.cargo/bin:$PATH"`
  (`tools/install.sh` sets it).
* Python scripts that patch files: write them with the Write tool. Heredocs
  mangle backslashes (`\\`, `\s`, `\N`).
* `ffmpeg` is on PATH (WinGet Gyan.FFmpeg).
* The user's monitors, left to right: ARZOPA 1920×1080, G27Q 2560×1440 (primary),
  Lenovo P27q-20 1440×2560 portrait. Top bars plus bottom docks on all three.
* `.scratch/` (git-ignored) is throwaway; anything worth keeping goes in `tools/`.
