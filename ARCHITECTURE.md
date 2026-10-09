# StatusNotch — how it works

A small Windows shell shaped like notches. By default each monitor gets a **top
status bar** (a black notch that morphs into a full-width bar on hover or a hot
corner) and a **bottom dock** (notches that grow in place: tasks, tray, quick
settings), and the Windows taskbar is hidden while it runs. Widgets can drop down
panels; an app **launcher** opens with a hotkey. Built with **Tauri v2**: a Rust
backend and a plain TypeScript + SVG frontend (no UI framework).

Guiding rules: keep it **minimal** (new extras must be optional and off by default),
**pure black** look, everything **configurable in `config.toml`** and exposed in the
settings window. Development rules, the catalogue of shared helpers, past rendering
bugs and how to build / test are in **`CLAUDE.md`**.

---

## 1. Big picture

```
Rust (src-tauri/src)                                 Frontend (src/)
──────────────────────────────────────────────────   ─────────────────────────────────
engine.rs   one thread: monitors, hover, docking,    main.ts      Overlay class: one per
            fullscreen, layout → emits events  ───►               monitor window, draws the
providers/  one thread each: temps, weather, media,              notch/bar SVG, mounts widgets
            tray host, tasks, system… → publish ───►  data.ts      subscribe(key, cb) data hub
osd/        volume keys, Core Audio, WMI,         geometry.ts  the shape (single SVG path)
            WMI brightness, keyboard layout  ──────►  widgets/*    widget + panel factories
config.rs   TOML model, defaults, resolve_layout     settings/*   the settings window
config_edit.rs  comment-preserving config writes ◄── (invoke)
```

* **One overlay window per surface per monitor** (`overlay-DISPLAYn-top`,
  `overlay-DISPLAYn-bottom`): transparent, always on top, click-through except where the
  frontend reports interactive regions. A subclass (`win::style_guard`) keeps them
  `WS_POPUP` + `WS_EX_TOOLWINDOW` without caption / system menu / minimize box: hidden
  from the taskbar and Alt+Tab, never minimized by Win+D / "show desktop" (a watchdog
  restores one anyway), and no title bar painted on activation (DWM non-client
  rendering is off too). Never use `skip_taskbar`.
* **Rust decides *where*, the frontend decides *how it looks*.** The engine computes
  layouts, notch positions, hover/open state; the overlay animates them with springs.
* **Config** lives in `%APPDATA%\StatusNotch\config.toml`, hot-reloaded on save.
  Built-in defaults: `src-tauri/default-config.toml` (written on first run).
* **Single instance:** a named mutex + events (`Local\StatusNotch.Quit`,
  `Local\StatusNotch.Settings`). Starting a new instance asks the old one to quit;
  `StatusNotch.exe --settings` opens settings in the running one.

## 2. File map

| Path | What it does |
|---|---|
| `src-tauri/src/main.rs` | Setup, plugin/command registration (`invoke_handler`), starts engine/providers/OSD/tray |
| `src-tauri/src/engine.rs` | Engine thread (~60 Hz): cursor polling → hover state machine (`Idle/Dwell/Open/Leave`) per surface, hot corners/edges, window docking ("undocked" notches), dock shrinking under maximized windows, fullscreen hiding, minimize watchdog, monitor changes, `build_layout` + `notch_positions` (unchanged layouts aren't resent); launcher open/close (`launcher_toggle`, closes when focus leaves); commands `get_layout`, `set_regions`, `set_notch_widths`, `set_content_height`, `fe_log`, `launcher_open/close` |
| `src-tauri/src/config.rs` | Serde model of `config.toml`, defaults, `surface_list()` / `resolve_surface()` (surface + per-monitor rule → notches & bar groups), `widget_enabled()`, legacy model support |
| `src-tauri/src/config_edit.rs` | `config_read`, `config_write(path, value)`, `config_write_many(ops)`, `config_reset` — `toml_edit`, keeps comments/CRLF, validates before writing |
| `src-tauri/src/providers/` | Data providers (see §5); `mod.rs` has the shared helpers (`publish`, `placed`, `widget_opts`, `rgba_json`, HTTP). `temps.rs` (LibreHardwareMonitor), `weather.rs` (Open-Meteo), `media.rs` (Windows SMTC), `systray.rs` (tray icon host), `tasks.rs` (app windows, pins, icons; activate/close/launch/reorder), `system.rs` (battery, brightness, network, and the volume the OSD watcher hands it, for quick settings), `audio.rs` (output devices, per-app volume), `extras.rs` (focus, netspeed, recycle bin, privacy), `claude.rs` (Claude Code usage from its session logs), `clipboard.rs` (history, in memory), `notifications.rs` (Windows notification database via winsqlite3) |
| `src-tauri/src/taskbar.rs` | Hides / restores the Windows taskbar (`[shell] hide_taskbar`), reserves screen space for always-shown bars (crash-safe marker) |
| `src-tauri/src/launcher.rs` | App index (Start menu shortcuts + Store apps), icons, launching, the global hotkey thread |
| `src-tauri/src/corners.rs` | Rounded screen corners as native layered windows (when not attached to the bars) |
| `src-tauri/src/dropzone.rs` | Files dropped on a bar (to pin them): WebView2 takes the drop, the page posts the files back (`postMessageWithAdditionalObjects`), this reads their paths |
| `src-tauri/src/osd/` | `mod.rs` OSD + keyboard-layout watch; `volume.rs` Core Audio; `brightness.rs` WMI |
| `src-tauri/src/keyhook.rs` | The low-level keyboard hook, in a helper process (`StatusNotch.exe --keyhook <pid>`, started by the app, exits with it): volume keys, and the Windows key opening the launcher (`[launcher] start_menu`). Talks to the app through named events |
| `src-tauri/src/actions.rs` | `run_action(action, target)`: open, settings, sleep, hibernate, lock, restart, shutdown, sign_out, open_config, open_log, open_config_folder |
| `src-tauri/src/settings.rs` | Opens the settings window; `monitors_list`, `lhm_sensors`, `system_time_format`, `app_version` |
| `src-tauri/src/presets.rs` | Presets = full config copies in `%APPDATA%\StatusNotch\presets\<name>.toml`. Shipped presets (`src-tauri/presets/*.toml`, the author's minus monitor rules, pins and weather city) are added once each on start (`presets/.shipped`). Applying a preset keeps your pins and weather city if it has none |
| `src-tauri/src/win.rs` | Win32 helpers: monitors/DPI, overlay styles, z-order, app windows (DWM frame bounds), fullscreen detection, keyboard layouts, and the small shared ones (UTF-16 strings, window title / class / process, bitmap pixels) |
| `src-tauri/src/{tray,autostart,instance,logfile}.rs` | Own tray icon menu, Run-key autostart, single instance, release log → `%APPDATA%\StatusNotch\statusnotch.log` |
| `src/main.ts` | `Overlay`: builds DOM from a `Layout`, springs, drops (notches/panels), OSD, keyboard peek, region reporting, notch width measuring |
| `src/geometry.ts` | `computeFrame()` → the one SVG path: notch profile (from the reference SVG), bar, panels, merging of overlapping notches |
| `src/spring.ts` | Damped spring used for every animation |
| `src/types.ts` | `Layout`, `NotchLayout`, … (mirror of Rust structs, camelCase) |
| `src/data.ts` | Data hub: `subscribe(key, cb)`; typed `DataMap` |
| `src/widgets/index.ts` | Widget + panel registries, `createWidget`, `createPanel`, `panelFor`, `Glyph`, `g()`, `el()`, `icon()`, `iconUrl()` |
| `src/widgets/tasks.ts` | Tasks: buttons per app (pinned + running on this monitor), drag to reorder, drop files to pin, hover popup / right-click panel |
| `src/settings/schema.ts` | Declarative settings pages (one entry per setting) |
| `src/settings/editors.ts` | Custom editors (placement drag & drop, notch pills, sensor pickers, menu editor, quick tiles, presets…) + config migrations |
| `src/settings/main.ts` | Settings window rendering, save/saveMany, migrations on load |
| `src/settings/dom.ts` | DOM helpers shared by both (`$`, `row`, `toggleSwitch`, …) |
| `tools/` | Install script and test tools (see `CLAUDE.md`) |

## 3. Runtime flow

1. `main.rs` loads config, starts **providers**, **OSD**, the **engine**, and the tray icon.
2. The engine creates one overlay window per monitor and emits **`notch://layout`**
   (a full `Layout`) to it. The page also calls `get_layout` on load.
3. `Overlay.buildDom(layout)` mounts the **bar** (three groups: left/center/right) and
   one **slot per notch**, creating widgets with `createWidget(name, layout.widgets[name])`.
4. Each frame the overlay builds `Bump`s (notches, open panels, OSD, keyboard peek),
   calls `computeFrame()` and sets the SVG path. It reports clickable rectangles with
   `set_regions` (the window is click-through elsewhere).
5. The engine polls the cursor: trigger zones (notch bodies, hot corners) → dwell →
   **`notch://state {expanded}`**; it toggles click-through and raises the window.
6. Notch content width is **measured by the frontend** (`set_notch_widths`); the engine
   re-positions notches (stacking from corners) and emits **`notch://positions`**.
7. Providers `publish(key, value)` → **`notch://data`** → `data.ts` → widget callbacks.

**Events (Rust → overlay):** `notch://layout`, `notch://state`, `notch://positions`,
`notch://data`, `notch://osd`, `notch://keyboard`, `notch://launcher`, `notch://drop`.

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
  edge; a bottom surface lays its content out from the window's bottom (only the
  SVG shape is mirrored, and the clip path via `mirrorPath`): never mirror boxes
  with text in them, they shimmer when their height changes. `mode = "always"`
  keeps a bar shown (optionally `reserve_space`). Hot zones (`hot_left/right/edge`,
  sizes) are per surface. Widget instances (`buttons#2`) let one widget type appear
  several times.

* **Open bar:** every enabled widget, in side order (notch widgets included).
* **Closed:** only notch widgets show, inside notches. With `[notch] merge = true`
  (default) notch widgets on the same side share **one notch** (ids `left`/`center`/`right`);
  otherwise one notch per widget (id = widget name).
* **Opening:** a notch widget is a single instance: it stays in its notch while the bar
  opens, and the bar's groups make room for it (`Overlay.layoutBar`; center widgets sit
  before / after a middle notch). A widget that looks different open / closed styles
  itself with `html.bar-open` (media keys).
* **Side notches** (`[notch.left|right]`): `hug` = sit in the corner (content keeps the
  bar's edge padding so nothing shifts on open), `avoid_windows` = slide to `offset` while
  a window's caption buttons are there, `offset`. Center: `offset`.
* **Per widget:** `hide_when_maximized = true` — the widget leaves its notch while a window
  covers it; a notch left empty retracts into the top edge.
* `combine_overlapping` draws touching notches as one shape (positions unchanged).
* **Monitor rules** (`[[monitors]]`, first match wins): `match = "*" | "primary" | "#2" |
  device | part of friendly name`, `enabled`, optional
  `surfaces = { top = { custom_layout = true, left, center, right, notch }, bottom = { enabled = false } }`.

## 5. Widgets

A widget is a **factory** returning a DOM element:

```ts
// src/widgets/index.ts
export interface Widget { el: HTMLElement; dispose?: () => void }
type Factory = (opts: Record<string, unknown>) => Widget;     // opts = [widgets.<name>] table
type PanelFactory = (opts, ctx: { close(): void; resize(): void }) => Widget;
```

Example (clock): reads `opts.format`, updates text on a timer, `dispose` clears the timer.
`opts` also carries `_device` (the overlay's monitor), used by tasks to list that
monitor's windows.
Widgets get live data with `subscribe("media", (m) => …)` (returns an unsubscribe — call it
in `dispose`). Root element class is `widget <name>`; style in `src/styles.css`.
Interactive bits need `pointer-events` (the overlay is click-through except reported regions).

**Panels** (dropdowns): registered in `panels`; `panelFor(widget)` says which panel a
widget opens (clock → `calendar`, others by same name). Clicking the widget in the open
bar drops the panel; panels auto-size.

Existing widgets: `clock` (+ `calendar` panel), `menu` (panel with actions), `tray`
(hosted tray icons; collapsed panel or expanded in the bar), `language` (keyboard
layout, click to switch), `media` (SMTC, keys + title, panel with art/seek), `weather`
(Open-Meteo, hourly panel), `temps` (LibreHardwareMonitor CPU/GPU, details panel),
`tasks` (app windows + pins), `quick` (volume/brightness/battery/network panel, output
devices, per-app volume, shortcut tiles), `buttons` (own action buttons, instances),
`launcher` (search field + results), `language`, and optional extras: `focus`,
`netspeed`, `recycle`, `privacy`, `claude`, `clipboard`, `notifications`.

### Adding a widget — checklist
1. `src/widgets/<name>.ts`: factory (and optional panel factory); register in
   `widgets` / `panels` in `src/widgets/index.ts`.
2. If it needs system/external data: a provider (below) + a typed entry in `DataMap`
   (`src/data.ts`).
3. Settings: add it to `WIDGETS` in `src/settings/editors.ts` (placement chips with
   their notch pill come for free) and a collapsible section in `schema.ts` with its
   options (`path: ["widgets", "<name>", "<option>"]`).
4. Defaults: a `[widgets.<name>]` table in `default-config.toml` (new extras:
   `enabled = false`).
5. Styles in `src/styles.css`.

### Adding a data provider (Rust)
`src-tauri/src/providers/<name>.rs` with a thread loop that idles unless
`placed(&app, "<name>")` (the widget is on some bar), reads options with `widget_opts`,
fetches, and calls `publish(&app, "<key>", json)` (deduplicated; broadcast as
`notch://data`). Start it in `providers::start`. Prefer Windows notifications to
polling where there are any. HTTP: `http_agent()` + `get_json()` (ureq, 5 s timeout). Commands
the widget calls go in a module and must be listed in `main.rs`'s `generate_handler!`.

## 6. Settings window

Generated from `src/settings/schema.ts`: pages → sections → fields. Field kinds:
`toggle`, `slider`, `number`, `text`, `select`, `color`, `list`, `autostart`, `action`,
`custom` (an editor from `editors.ts`). `path` is where the value lives in config.toml;
it can be a function of the config returning `null` to hide the field. Writes go through
`config_write`/`config_write_many` (comment-preserving, validated), and the engine
hot-reloads. Pages: General (startup, behaviour, hot corners), Widgets (Global/per-monitor
layout, placement drag & drop, Notches, per-widget sections), Appearance, Animation,
Hover, OSD, Presets, About. Config format changes need a migration in `editors.ts`
(`migrationOps`, `migrationOpsV3`, `V4`, `V5`, `migrationOpsDisabledSpots`), run in
order by `settings/main.ts` when settings opens.

## 7. Config reference (top-level tables)

| Table | Holds |
|---|---|
| `[timing]` | `poll_ms`, `dwell_ms`, `grace_ms`, `edge_px`, `hot_corner` (size), `hot_top_left/right/edge` |
| `[general]` | `hide_in_fullscreen`, `rounded_corners`, `corner_radius`, `corners_attached` |
| `[shell]` | `hide_taskbar` |
| `[launcher]` | `enabled`, `hotkey` |
| `[theme]` | `background`, `opacity`, `text`, `dim_opacity`, `accent`, `warn`, `font_size` |
| `[geometry]` | `expanded_height`, `notch_height`, `concave_radius`, `notch_corner_radius`, `corner_radius`, panel radii |
| `[animation]` | spring stiffness/damping per motion, `mass`, `style = "drop" | "spread"` |
| `[osd]` | `enabled`, `notch`, size, `duration_ms`, `volume_step`, `replace_volume_keys`, `brightness`, `keyboard` |
| `[bar]` | `padding`, `gap` |
| `[sizes]` | `tiny`, `small`, `medium`, `large` |
| `[surfaces.<id>]`, `[notch]`, `[[monitors]]` | see §4 (`[layout]` is the pre-0.2 form, migrated) |
| `[widgets.<name>]` | `enabled` + widget options (e.g. `clock.format`, `media.max_width`/`scroll`, `weather.location/latitude/longitude/units`, `temps.url` + sensor ids, `menu.items`, `tray.mode/hide`, `tasks.pins/pinning/labels/icons/monitor/dim_minimized`, `buttons.items`) |

## 8. OSD, tray, Windows specifics

* **Keyboard hook:** in its own helper process (keyhook.rs). A hook inside the app
  wasn't run while one of our windows had the focus (and neither were the hooks
  after it: other apps' shortcuts died in our windows). The helper has no window.
  With `[launcher] start_menu` it holds a physical Windows key back: another key
  next replays Win + that key (shortcuts work); released alone, it presses a
  private hotkey (Ctrl+Alt+Shift+F24) the app registers, because a hotkey lets the
  app take the focus. Simulated Win presses (PowerToys remaps) pass untouched.
* **OSD:** the keyboard helper swallows volume keys (when `replace_volume_keys`),
  sets volume via Core Audio, and emits `notch://osd`; the notch holding `osd.notch`
  grows into a pill. Its Core Audio volume callback is also quick settings' only source
  for the volume (`system::volume_changed`). Brightness (Fn keys) is watched through WMI. Keyboard-layout
  changes pop a small "peek" notch at the language widget.
* **Tray hosting:** a hidden `Shell_TrayWnd` receives apps' `Shell_NotifyIcon` calls,
  forwards them to Explorer, and publishes icons (`tray` data key). It must stay *quiet*
  (no z-order churn) — a noisy host once cost Explorer ~50% CPU.
* **Never force-kill** StatusNotch (`taskkill /F`) — the graceful quit hands the tray back
  to Explorer. The installer's hooks use the quit event.
* Fullscreen apps hide the overlay (shell processes excluded, 500 ms debounce).
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
npm run build                                   # tsc + vite → dist/
cd src-tauri && cargo test --features tauri/custom-protocol
cargo build --release --features tauri/custom-protocol   # → target/release/statusnotch.exe
npx tauri build                                 # NSIS installer → target/release/bundle/nsis/
```

Installer: per-user NSIS (`tauri.conf.json` → `bundle`), hooks in
`src-tauri/windows/hooks.nsh`. For a release: bump `version` in `tauri.conf.json`,
`Cargo.toml` and `package.json`; make `default-config.toml` match the **Default**
preset (minus its `[[monitors]]` rules) — that's what ships.
