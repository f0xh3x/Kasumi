# Kasumi — how it works

A small Windows shell shaped like notches. By default each monitor gets a **top
status bar** (a black notch that morphs into a full-width bar on hover or a hot
corner) and a **bottom dock** (notches that grow in place: tasks, tray, quick
settings), and the Windows taskbar is hidden while it runs. Widgets can drop down
panels; an app **launcher** opens with a hotkey. Built with **Tauri v2**: a Rust
backend that also draws the bars natively (Direct2D / DirectWrite into layered
windows), and one web page, the settings window (plain TypeScript, no framework).

Guiding rules: keep it **minimal** (new extras must be optional and off by default),
**pure black** look, everything **configurable in `config.toml`** and exposed in the
settings window. Development rules, the catalogue of shared helpers, past rendering
bugs and how to build / test are in **`CLAUDE.md`**.

Up to 0.6 the bars could also be WebView2 pages (`src/main.ts`, `src/widgets/`):
removed in 0.7.0; that code is in git at `v0.6.7`.

---

## 1. Big picture

```
engine.rs     one thread: monitors, hover, docking,      native/ (one thread)
              fullscreen, layout ──── ui.rs ──────────►  Surface per bar: the shape
providers/    one thread each: temps, weather, media,    (geometry.rs), widgets and
              tray host, tasks, system… publish ── ui ►  panels (widgets/*), pop-ups,
osd/          volume keys, Core Audio, WMI brightness,   tooltips, menus (ctxmenu/),
              keyboard layout, scroll position ── ui ──► scroll bar, Alt + Tab
config.rs     TOML model, defaults, resolve_layout
config_edit.rs  comment-preserving writes ◄── settings page (src/settings, invoke)
```

* **One window per surface per monitor** (`overlay-DISPLAYn-top`,
  `overlay-DISPLAYn-bottom`), made by the native thread: layered, topmost, tool
  windows (not in the taskbar or Alt + Tab), click-through except where a surface
  reports interactive regions; each sizes itself to what shows every frame.
* **The engine decides *where*, the surface *how it looks*.** The engine computes
  layouts, notch positions, hover / open state; the surface animates them with
  springs and measures its notches' content (`native_widths`) for the engine.
* **Config** lives in `%APPDATA%\Kasumi\config.toml`, hot-reloaded on save.
  Built-in defaults: `src-tauri/default-config.toml` (written on first run).
* **Single instance:** a named mutex + events (`Local\Kasumi.Quit`,
  `Local\Kasumi.Settings`). Starting a new instance asks the old one to quit;
  `Kasumi.exe --settings` opens settings in the running one.
* **Helper processes:** `--keyhook` (keyboard hook, and the mouse wheel while the
  scroll pop-up is on), `--menu` (Explorer's right-clicks with Kasumi's context
  menu on), `--icons` (app icons, so the shell's imaging stack stays out of Kasumi).

## 2. File map

| Path | What it does |
|---|---|
| `src-tauri/src/main.rs` | Setup, the settings page's commands (`invoke_handler`), starts the native renderer, engine, providers, OSD, helpers, tray |
| `src-tauri/src/engine.rs` | Engine thread (~60 Hz): cursor polling → hover state machine (`Idle/Dwell/Open/Leave`) per surface, hot corners/edges, window docking ("undocked" notches), dock shrinking under maximized windows, fullscreen hiding, monitor changes, `build_layout` + `notch_positions` (unchanged layouts aren't resent); launcher open/close (`launcher_toggle`, closes when focus leaves); `fe_log` |
| `src-tauri/src/ui.rs` | The seam: everything for a bar goes to the native thread through it |
| `src-tauri/src/native/` | The renderer: `mod.rs` (the thread, `Msg`, `Spring`), `surface.rs` (a bar or dock: shape, widgets, panels, hover, clicks, drops), `geometry.rs` (the shape), `gfx.rs` (Direct2D), `widgets/*` (each widget and panel), `popup.rs` (pop-ups out of a notch), `tooltip.rs`, `floating.rs` (Alt + Tab centred), `scrollbar.rs` (scroll position), `apptips.rs` (other apps' tooltips), `ctxmenu/` (Kasumi's menus), `drop.rs` (files dropped on the dock), `snapshot.rs` (`--native-snapshot`) |
| `src-tauri/src/config.rs` | Serde model of `config.toml`, defaults, `surface_list()` / `resolve_surface()` (surface + per-monitor rule → notches & bar groups), `widget_enabled()`, legacy model support |
| `src-tauri/src/config_edit.rs` | `config_read`, `config_write(path, value)`, `config_write_many(ops)`, `config_reset` — `toml_edit`, keeps comments/CRLF, validates before writing |
| `src-tauri/src/providers/` | Data providers (§5); `mod.rs` has the shared helpers (`publish`, `placed`, `widget_opts`, `rgba_json`, HTTP). `temps.rs`, `weather.rs`, `media.rs` (SMTC), `systray.rs` (tray icon host), `tasks.rs` (app windows, pins, icons), `system.rs` (battery, brightness, network, volume), `audio.rs`, `extras.rs` (focus, netspeed, disks, privacy), `claude.rs`, `clipboard.rs`, `notifications.rs`, `switches.rs` (Wi-Fi / Bluetooth radios, dark mode), `batteries.rs` (Bluetooth devices' and the laptop's), `komorebi.rs` |
| `src-tauri/src/taskbar.rs` | Hides / restores the Windows taskbar (`[shell] hide_taskbar`), reserves screen space for always-shown bars (crash-safe marker) |
| `src-tauri/src/launcher.rs` | The launcher's index (Apps folder, Control Panel, PATH programs, Settings pages), icons, launching, Start's pins, the global hotkey thread |
| `src-tauri/src/corners.rs` | Rounded screen corners as layered windows (when not attached to the bars) |
| `src-tauri/src/osd/` | `mod.rs` OSD + keyboard-layout watch; `volume.rs` Core Audio; `brightness.rs` WMI; `fileops.rs` file copy progress; `scroll.rs` scroll position (UI Automation / scroll bars) |
| `src-tauri/src/keyhook.rs` | The input hooks' helper process (`Kasumi.exe --keyhook <pid>`): volume keys, the Windows key opening the launcher, Win + 1..9, Alt + Tab, Win + Space, the wheel for the scroll pop-up. Talks to the app through named events |
| `src-tauri/src/actions.rs` | `run_action(action, target)`: open, settings, sleep, hibernate, lock, restart, shutdown, sign_out, open_config, open_log, open_config_folder |
| `src-tauri/src/settings.rs` | Opens the settings / setup / notes windows (WebView2); `monitors_list`, `lhm_sensors`, `fonts_list`, `app_version` |
| `src-tauri/src/presets.rs` | Presets = full config copies in `%APPDATA%\Kasumi\presets\<name>.toml`; shipped presets added once each |
| `src-tauri/src/win.rs` | Win32 helpers: monitors/DPI, z-order, app windows (DWM frame bounds), fullscreen detection, keyboard layouts, registry, the small shared ones |
| `src-tauri/src/{tray,autostart,instance,logfile,updater,legacy}.rs` | Own tray icon menu, Run-key autostart, single instance, log → `%APPDATA%\Kasumi\kasumi.log`, updates from GitHub, the move-over from StatusNotch |
| `settings.html`, `src/settings/*` | The settings window: `schema.ts` (declarative pages), `editors.ts` (custom editors, migrations), `main.ts` (rendering, search, saving), `setup.ts` (first-time setup, update notes), `ctxmenu-editor.ts`, `dom.ts` |
| `src/quick-tiles.ts`, `src/power-actions.ts` | Catalogues the settings page uses (in step with the native lists) |
| `tools/` | Install script and test tools (see `CLAUDE.md`) |

## 3. Runtime flow

1. `main.rs` loads config, starts the **native thread**, the **engine**, **providers**,
   **OSD**, the helpers and the tray icon.
2. The engine has the native thread make one window per surface per monitor
   (`ui::create`) and sends it a full `Layout` (`ui::layout`).
3. The surface builds its widgets (`widgets::create(name, opts)`) and panels, draws
   the shape (`geometry::compute`) and reports its interactive regions and notch
   widths back to the engine.
4. The engine polls the cursor: trigger zones (notch bodies, hot corners) → dwell →
   `ui::state(expanded)`; it toggles click-through and raises the window.
5. Measured notch widths → the engine re-positions notches (stacking from corners) →
   `ui::positions`.
6. Providers `publish(key, value)` → `ui::data` → every surface's widgets and panels
   that list the key (`keys()`), and the native thread keeps it for surfaces made
   later.
7. Pop-ups (`osd::emit`), the launcher, dropped files: `ui::event(label, name, payload)`.

## 4. Surfaces, layout & notch model

```toml
[surfaces.top]                 # a [[monitors]] rule may override: surfaces = { top = {...} }
mode   = "bar"                 # bar | dock | always
left   = ["menu"]
center = ["clock"]
right  = ["buttons"]
notch  = ["clock"]             # which of those also show as a notch while the bar is closed

[surfaces.bottom]              # the dock
edge = "bottom"
mode = "dock"                  # notches only; on hover they grow in place
rest = "small"                 # [sizes]: tiny 12 | small 30 | medium 45 | large 60
open = "medium"
center = ["tasks", "tray", "quick"]
```

* **Surfaces** are drawn the same way. Positions are distances from the surface's
  edge; a bottom surface mirrors the shape and its panels' parts. `mode = "always"`
  keeps a bar shown (optionally `reserve_space`). Hot zones (`hot_left/right/edge`,
  sizes) are per surface. Widget instances (`buttons#2`) let one widget type appear
  several times.
* **Open bar:** every enabled widget, in side order (notch widgets included).
* **Closed:** only notch widgets show, inside notches. With `[notch] merge = true`
  (default) notch widgets on the same side share **one notch** (ids `left`/`center`/`right`);
  otherwise one notch per widget (id = widget name).
* **Opening:** a notch widget stays in its notch while the bar opens, and the bar's
  groups make room for it.
* **Side notches** (`[notch.left|right]`): `hug` = sit in the corner, `avoid_windows` =
  slide to `offset` while a window's caption buttons are there, `offset`. Center: `offset`.
* **Per widget:** `hide_when_maximized = true` — the widget leaves its notch while a window
  covers it; a notch left empty retracts into the top edge.
* `combine_overlapping` draws touching notches as one shape (positions unchanged).
* **Monitor rules** (`[[monitors]]`, first match wins): `match = "*" | "primary" | "#2" |
  device | part of friendly name`, `enabled`, optional
  `surfaces = { top = { custom_layout = true, left, center, right, notch }, bottom = { enabled = false } }`.

## 5. Widgets

A widget is a Rust type implementing `native::widgets::Widget` (`width`, `draw`,
optional `keys` + `data`, `click`, `tick`/`wake`, `tooltip`, `act`…); a panel
implements `Panel` (`size`, `draw`, `hover`, `click`, `tooltip`…). Options come as
`Opts` (the `[widgets.<name>]` table plus resolved animation flags, fonts, the
surface and monitor). Most bar widgets are a row of `Piece`s (`row_width` /
`row_draw`).

Existing widgets: `clock` (+ `calendar` panel, timers), `menu`, `tray`, `language`,
`media`, `weather`, `temps`, `tasks`, `quick`, `buttons` (instances), `launcher`,
`batteries`, `workspaces` (Komorebi), and extras: `focus`, `netspeed`, `disk`,
`privacy`, `claude`, `clipboard`, `notifications`.

### Adding a widget — checklist
1. `src-tauri/src/native/widgets/<name>.rs` (or an existing file of small ones): the
   widget, and its panel if it has one; register them in `widgets/mod.rs` (`create`,
   `create_panel`, `panel_for`).
2. If it needs system/external data: a provider (below), publishing a key the widget
   lists in `keys()`.
3. Settings: add it to `WIDGETS` and `SECTION` in `src/settings/editors.ts`
   (placement chips come for free) and a collapsible section in `schema.ts` with its
   options (`path: ["widgets", "<name>", "<option>"]`).
4. Defaults: new extras are off (not placed); options default in code.
5. Check it off screen with `--native-snapshot` (a test config placing it, test data).

### Adding a data provider (Rust)
`src-tauri/src/providers/<name>.rs` with a thread loop that idles unless
`placed(&app, "<name>")` (the widget is on some bar) or its option is on, reads
options with `widget_opts`, fetches, and calls `publish(&app, "<key>", json)`
(deduplicated). Start it in `providers::start`. Prefer Windows notifications to
polling where there are any. HTTP: `http_agent()` + `get_json()` (ureq, 5 s timeout).

## 6. Settings window

Generated from `src/settings/schema.ts`: pages → sections → fields. Field kinds:
`toggle`, `slider`, `number`, `text`, `select`, `color`, `list`, `autostart`, `action`,
`custom` (an editor from `editors.ts`). `path` is where the value lives in config.toml;
it can be a function of the config returning `null` to hide the field. Writes go through
`config_write`/`config_write_many` (comment-preserving, validated), and the engine
hot-reloads. A search field in the sidebar lists matching settings with their
controls. Config format changes need a migration in `editors.ts` (`migrationOps`,
`migrationOpsV3`, `V4`, `V5`, `migrationOpsDisabledSpots`, `migrationOpsRemovedWidgets`),
run in order by `settings/main.ts` when settings opens.

## 7. Config reference (top-level tables)

| Table | Holds |
|---|---|
| `[timing]` | `poll_ms`, `dwell_ms`, `grace_ms`, `edge_px`, `hot_corner` (size), `hot_top_left/right/edge` |
| `[general]` | `hide_in_fullscreen`, `rounded_corners`, `corner_radius`, `corners_attached`, `tooltips`, `app_tooltips` |
| `[shell]` | `hide_taskbar` |
| `[launcher]` | `enabled`, `hotkey`, `start_menu`, `fullscreen` |
| `[theme]` | `background`, `opacity`, `text`, `dim_opacity`, `accent`, `warn`, `font_size`, fonts |
| `[geometry]` | `expanded_height`, `notch_height`, `concave_radius`, `notch_corner_radius`, `corner_radius`, panel radii |
| `[animation]` | spring stiffness/damping per motion, `mass`, `style = "drop" | "spread"` |
| `[osd]` | `enabled`, screens / spots, size, `duration_ms`, `volume_step`, `replace_volume_keys`, `brightness`, `keyboard`, `caps_lock`, `file_progress`, `scroll` (+ `scroll_length`, `scroll_width`), notifications |
| `[bar]` | `padding`, `gap` |
| `[sizes]` | `tiny`, `small`, `medium`, `large` |
| `[surfaces.<id>]`, `[notch]`, `[[monitors]]` | see §4 (`[layout]` is the pre-0.2 form, migrated) |
| `[context_menu]`, `[bar_menu]`, `[launcher_menu]` | Kasumi's menus and their layouts |
| `[widgets.<name>]` | `enabled` + widget options (e.g. `clock.format/timers`, `launcher.results/calculator/settings/pinned/clipboard/user/buttons/buttons_position`, `quick.tiles/switches/devices`, `batteries.show/laptop`, `tasks.pins`, `buttons.items`) |

## 8. OSD, tray, Windows specifics

* **Input hooks:** in their own helper process (keyhook.rs). A hook inside the app
  wasn't run while one of our windows had the focus (and neither were the hooks
  after it: other apps' shortcuts died in our windows). The helper has no window.
  With `[launcher] start_menu` it holds a physical Windows key back: another key
  next replays Win + that key (shortcuts work); released alone, it presses a
  private hotkey (Ctrl+Alt+Shift+F24) the app registers, because a hotkey lets the
  app take the focus. Simulated Win presses (PowerToys remaps) pass untouched.
* **OSD:** the keyboard helper swallows volume keys (when `replace_volume_keys`),
  sets volume via Core Audio, and `osd::emit` sends the pop-up; a notch grows into a
  pill. Its Core Audio volume callback is also quick settings' only source for the
  volume (`system::volume_changed`). Brightness (Fn keys) is watched through WMI.
  Keyboard-layout changes pop a small "peek" notch at the language widget.
* **Tray hosting:** a hidden `Shell_TrayWnd` receives apps' `Shell_NotifyIcon` calls,
  forwards them to Explorer, and publishes icons (`tray` data key). It must stay *quiet*
  (no z-order churn) — a noisy host once cost Explorer ~50% CPU.
* **Never force-kill** Kasumi (`taskkill /F`) — the graceful quit hands the tray back
  to Explorer. The installer's hooks use the quit event.
* Fullscreen apps hide the bars (shell processes excluded, 500 ms debounce).
* Window rects use `DWMWA_EXTENDED_FRAME_BOUNDS` (maximized windows' invisible borders
  otherwise bleed into neighbour monitors).
* **Tasks:** windows follow Alt+Tab rules; minimized windows count for the monitor
  they'll be restored to, and a window stays listed while it's briefly cloaked
  (minimize / restore animations). Activating borrows the foreground app's input
  queue (`AttachThreadInput`) so Windows lets the switch happen.
* **Hidden taskbar:** restored on quit and after a crash (next start). Reserving
  space for an always-shown bar sets the work area without a broadcast (that would
  re-show the taskbar).

## 9. Build & release

`tools/install.sh` does the build + install + checks in one go (see `CLAUDE.md`). By hand:

```bash
npm install
npm run build                                   # tsc + vite → dist/ (the settings page)
cd src-tauri && cargo test --features tauri/custom-protocol
cargo build --release --features tauri/custom-protocol   # → target/release/kasumi.exe
npx tauri build                                 # NSIS installer → target/release/bundle/nsis/
```

Installer: per-user NSIS (`tauri.conf.json` → `bundle`), hooks in
`src-tauri/windows/hooks.nsh`. For a release: bump `version` in `tauri.conf.json`,
`Cargo.toml` and `package.json`; make `default-config.toml` match the **Default**
preset (minus its `[[monitors]]` rules) — that's what ships.
