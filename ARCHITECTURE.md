# StatusNotch — how it works

A Windows status bar shaped like a notch. A black notch sits at the top of each
monitor; hovering it (or a hot corner) morphs it into a full-width bar, and widgets
in the bar can drop down panels. Built with **Tauri v2**: a Rust backend and a plain
TypeScript + SVG frontend (no UI framework).

Guiding rules: keep it **minimal** (new extras must be optional and off by default),
**pure black** look, everything **configurable in `config.toml`** and exposed in the
settings window.

---

## 1. Big picture

```
Rust (src-tauri/src)                                 Frontend (src/)
──────────────────────────────────────────────────   ─────────────────────────────────
engine.rs   one thread: monitors, hover, docking,    main.ts      Overlay class: one per
            fullscreen, layout → emits events  ───►               monitor window, draws the
providers/  one thread each: temps, weather,                      notch/bar SVG, mounts widgets
            media (SMTC), systray host → publish ──►  data.ts      subscribe(key, cb) data hub
osd/        keyboard hook (volume keys), Core Audio,  geometry.ts  the shape (single SVG path)
            WMI brightness, keyboard layout  ──────►  widgets/*    widget + panel factories
config.rs   TOML model, defaults, resolve_layout     settings/*   the settings window
config_edit.rs  comment-preserving config writes ◄── (invoke)
```

* **One overlay window per monitor** (`overlay-DISPLAYn`): transparent, always on top,
  click-through except where the frontend reports interactive regions. Hidden from the
  taskbar/Alt+Tab (a subclass forces `WS_EX_TOOLWINDOW`; never use `skip_taskbar`).
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
| `src-tauri/src/engine.rs` | Engine thread (~60 Hz): cursor polling → hover state machine (`Idle/Dwell/Open/Leave`), hot corners, window docking ("undocked" notches), fullscreen hiding, monitor changes, `build_layout` + `notch_positions`; commands `get_layout`, `set_regions`, `set_notch_widths`, `set_content_height`, `fe_log` |
| `src-tauri/src/config.rs` | Serde model of `config.toml`, defaults, `resolve_layout()` (layout + per-monitor rule → notches & bar groups), `widget_enabled()`, legacy model support |
| `src-tauri/src/config_edit.rs` | `config_read`, `config_write(path, value)`, `config_write_many(ops)`, `config_reset` — `toml_edit`, keeps comments/CRLF, validates before writing |
| `src-tauri/src/providers/` | Data providers (see §5): `temps.rs` (LibreHardwareMonitor), `weather.rs` (Open-Meteo), `media.rs` (Windows SMTC), `systray.rs` (tray icon host) |
| `src-tauri/src/osd/` | `mod.rs` low-level keyboard hook + keyboard-layout watch; `volume.rs` Core Audio; `brightness.rs` WMI |
| `src-tauri/src/actions.rs` | `run_action(action, target)`: open, settings, sleep, hibernate, lock, restart, shutdown, sign_out, open_config, open_log, open_config_folder |
| `src-tauri/src/settings.rs` | Opens the settings window; `monitors_list`, `lhm_sensors`, `system_time_format`, `app_version` |
| `src-tauri/src/presets.rs` | Presets = full config copies in `%APPDATA%\StatusNotch\presets\<name>.toml` |
| `src-tauri/src/win.rs` | Win32 helpers: monitors/DPI, overlay styles, z-order, app windows (DWM frame bounds), fullscreen detection, keyboard layouts |
| `src-tauri/src/{tray,autostart,instance,logfile}.rs` | Own tray icon menu, Run-key autostart, single instance, release log → `%APPDATA%\StatusNotch\statusnotch.log` |
| `src/main.ts` | `Overlay`: builds DOM from a `Layout`, springs, drops (notches/panels), OSD, keyboard peek, glide animation, region reporting, notch width measuring |
| `src/geometry.ts` | `computeFrame()` → the one SVG path: notch profile (from the reference SVG), bar, panels, merging of overlapping notches |
| `src/spring.ts` | Damped spring used for every animation |
| `src/types.ts` | `Layout`, `NotchLayout`, … (mirror of Rust structs, camelCase) |
| `src/data.ts` | Data hub: `subscribe(key, cb)`; typed `DataMap` |
| `src/widgets/index.ts` | Widget + panel registries, `createWidget`, `createPanel`, `panelFor`, `Glyph`, `el()`, `icon()` |
| `src/settings/schema.ts` | Declarative settings pages (one entry per setting) |
| `src/settings/editors.ts` | Custom editors (placement drag & drop, notch toggles, sensor pickers, menu editor, presets…) + config migrations |
| `src/settings/main.ts` | Settings window rendering, save/saveMany, migrations on load |

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
`notch://data`, `notch://osd`, `notch://keyboard`.

## 4. Layout & notch model

```toml
[layout]                       # global; a [[monitors]] rule may override with layout = {...}
left   = ["menu", "language"]
center = ["clock"]
right  = ["tray", "weather", "media", "temps"]
notch  = ["clock"]             # which of those also show as a notch while the bar is closed
```

* **Open bar:** every enabled widget, in side order (notch widgets included).
* **Closed:** only notch widgets show, inside notches. With `[notch] merge = true`
  (default) notch widgets on the same side share **one notch** (ids `left`/`center`/`right`);
  otherwise one notch per widget (id = widget name).
* **Opening:** each notch widget **glides** from its notch spot to its bar twin's spot,
  then the bar copy takes over (`Overlay.glide`). Notch slot content gets `inNotch: true`
  in its options, so a widget can render a compact variant (media hides its keys).
* **Side notches** (`[notch.left|right]`): `hug` = sit in the corner (content keeps the
  bar's edge padding so nothing shifts on open), `avoid_windows` = slide to `offset` while
  a window's caption buttons are there, `offset`. Center: `offset`.
* **Per widget:** `hide_when_maximized = true` — the widget leaves its notch while a window
  covers it; a notch left empty retracts into the top edge.
* `combine_overlapping` draws touching notches as one shape (positions unchanged).
* **Monitor rules** (`[[monitors]]`, first match wins): `match = "*" | "primary" | "#2" |
  device | part of friendly name`, `enabled`, optional `layout = { left, center, right, notch }`.

## 5. Widgets

A widget is a **factory** returning a DOM element:

```ts
// src/widgets/index.ts
export interface Widget { el: HTMLElement; dispose?: () => void }
type Factory = (opts: Record<string, unknown>) => Widget;     // opts = [widgets.<name>] table
type PanelFactory = (opts, ctx: { close(): void; resize(): void }) => Widget;
```

Example (clock): reads `opts.format`, updates text on a timer, `dispose` clears the timer.
Widgets get live data with `subscribe("media", (m) => …)` (returns an unsubscribe — call it
in `dispose`). Root element class is `widget <name>`; style in `src/styles.css`.
Interactive bits need `pointer-events` (the overlay is click-through except reported regions).

**Panels** (dropdowns): registered in `panels`; `panelFor(widget)` says which panel a
widget opens (clock → `calendar`, others by same name). Clicking the widget in the open
bar drops the panel; panels auto-size.

Existing widgets: `clock` (+ `calendar` panel), `menu` (panel with actions), `tray`
(hosted tray icons panel), `language` (keyboard layout, click to switch), `media`
(SMTC, keys + title, panel with art/seek), `weather` (Open-Meteo, hourly panel),
`temps` (LibreHardwareMonitor CPU/GPU, details panel).

### Adding a widget — checklist
1. `src/widgets/<name>.ts`: factory (and optional panel factory); register in
   `widgets` / `panels` in `src/widgets/index.ts`. Support `opts.inNotch` if it needs
   a compact notch variant.
2. If it needs system/external data: a provider (below) + a typed entry in `DataMap`
   (`src/data.ts`).
3. Settings: add it to `WIDGETS` in `src/settings/editors.ts` (placement chips and
   "Own notch" toggle come for free) and a collapsible section in `schema.ts` with its
   options (`path: ["widgets", "<name>", "<option>"]`).
4. Defaults: a `[widgets.<name>]` table in `default-config.toml` (new extras:
   `enabled = false`).
5. Styles in `src/styles.css`.

### Adding a data provider (Rust)
`src-tauri/src/providers/<name>.rs` with `pub fn run(app: AppHandle)` — a loop that
checks `widget_enabled(&app, "<name>")`, reads options with `widget_opts`, fetches, and
calls `publish(&app, "<key>", json)` (deduplicated; broadcast as `notch://data`). Spawn it
in `providers::start`. HTTP: `http_agent()` + `get_json()` (ureq, 5 s timeout). Commands
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
(`migrationOps`, `migrationOpsV3`), run when settings opens.

## 7. Config reference (top-level tables)

| Table | Holds |
|---|---|
| `[timing]` | `poll_ms`, `dwell_ms`, `grace_ms`, `edge_px`, `hot_corner` (size), `hot_top_left/right/edge` |
| `[general]` | `hide_in_fullscreen` |
| `[theme]` | `background`, `text`, `dim_opacity`, `accent`, `warn`, `font_size` |
| `[geometry]` | `expanded_height`, `notch_height`, `concave_radius`, `notch_corner_radius`, `corner_radius`, panel radii |
| `[animation]` | spring stiffness/damping per motion, `mass`, `style = "drop" | "spread"` |
| `[osd]` | `enabled`, `notch`, size, `duration_ms`, `volume_step`, `replace_volume_keys`, `brightness`, `keyboard` |
| `[bar]` | `padding`, `gap` |
| `[layout]`, `[notch]`, `[[monitors]]` | see §4 |
| `[widgets.<name>]` | `enabled` + widget options (e.g. `clock.format`, `media.max_width`/`scroll`, `weather.location/latitude/longitude/units`, `temps.url` + sensor ids, `menu.items`, `tray.hide`) |

## 8. OSD, tray, Windows specifics

* **OSD:** a low-level keyboard hook swallows volume keys (when `replace_volume_keys`),
  sets volume via Core Audio, and emits `notch://osd`; the notch holding `osd.notch`
  grows into a pill. Brightness (Fn keys) is watched through WMI. Keyboard-layout
  changes pop a small "peek" notch at the language widget.
* **Tray hosting:** a hidden `Shell_TrayWnd` receives apps' `Shell_NotifyIcon` calls,
  forwards them to Explorer, and publishes icons (`tray` data key). It must stay *quiet*
  (no z-order churn) — a noisy host once cost Explorer ~50% CPU.
* **Never force-kill** StatusNotch (`taskkill /F`) — the graceful quit hands the tray back
  to Explorer. The installer's hooks use the quit event.
* Fullscreen apps hide the overlay (shell processes excluded, 500 ms debounce).
* Window rects use `DWMWA_EXTENDED_FRAME_BOUNDS` (maximized windows' invisible borders
  otherwise bleed into neighbour monitors).

## 9. Build & release

```bash
npm install
npm run build                                   # tsc + vite → dist/
cd src-tauri && cargo test --features tauri/custom-protocol
cargo build --release --features tauri/custom-protocol   # → target/release/statusnotch.exe
npx tauri build                                 # NSIS installer → target/release/bundle/nsis/
```

Installer: per-user NSIS (`tauri.conf.json` → `bundle`), hooks in
`src-tauri/windows/hooks.nsh`. Bump `version` in `tauri.conf.json` for releases.
