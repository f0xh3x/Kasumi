# StatusNotch as a modular shell — plan

Goal: grow StatusNotch from a top status bar into a small, modular shell with a
**top status bar** and a **bottom dock** by default, both built from the same parts,
plus the widgets needed to live without the Windows taskbar: **tasks**, **quick
settings** and a **launcher**. Same philosophy as today: minimal, pure black,
everything optional and configurable, nothing in Windows replaced or broken.

Safety net: `main` is tagged **`v0.1.0`** (the working app). Work happens on the
**`shell`** branch. Binaries, installer, config and presets are copied to
`C:\Users\Kip\dev\StatusNotch-backups\v0.1.0\` (with a git bundle of the source).

---

## 1. The core idea: surfaces

Today there is exactly one thing per monitor: the top overlay. The change is to
make that a **surface**, and allow several per monitor. A surface is defined by:

| Property | Values | Meaning |
|---|---|---|
| `edge` | `top` · `bottom` | which screen edge it hangs from (bottom = mirrored) |
| `mode` | `bar` · `dock` | **bar**: notches open into the full-width bar (today's behaviour). **dock**: notch only — on hover the notches grow in place, no full-width bar |
| `rest` | `tiny` · `small` · `medium` | size while idle |
| `open` | `small` · `medium` | size while hovered / open |
| `left` `center` `right` `notch` | widget lists | same as today's `[layout]` |

Named sizes are heights, adjustable in `[sizes]`: **tiny = 12 px** (shape only, no
content — a hover handle), **small = 30 px** (today's notch), **medium = 60 px**.

The two defaults are just two surfaces with different settings:

```toml
[surfaces.top]          # status bar — exactly today's behaviour
edge = "top"
mode = "bar"
rest = "small"
open = "small"
left = ["menu"]
center = ["clock"]
right = ["tray"]
notch = ["clock"]

[surfaces.bottom]       # dock
edge = "bottom"
mode = "dock"
rest = "small"          # "tiny" = 12 px handle that opens on hover
open = "medium"         # twice as tall on hover
center = ["launcher", "tasks", "quick"]
```

So "a status bar or a dock" is one setting (`mode`), "top or bottom" is another
(`edge`), and any widget can be dragged between surfaces. Someone who wants tasks
in the top bar and no dock moves `tasks` up and sets `bottom.enabled = false`.

**Why this shape:** everything that works today (notch geometry, springs, glide,
panels, merging, hugging, hide-under-windows, presets, settings) is reused per
surface instead of being duplicated for a "dock feature". The status bar becomes
`surfaces.top` with no behaviour change, which keeps the refactor testable: phase 1
must look and feel identical to v0.1.0.

## 2. Widgets that adapt to size

Widgets get a small context next to their options:

```ts
type Factory = (opts: Options, ctx: WidgetContext) => Widget;
interface WidgetContext {
  surface: "top" | "bottom" | string;
  edge: "top" | "bottom";
  /** Called now and whenever the surface morphs; t = 0 at rest size, 1 at open size. */
  onSize(cb: (s: { rest: Size; open: Size; t: number; height: number }) => void): () => void;
}
```

* Most widgets ignore it (existing factories keep working unchanged).
* The surface root also exposes CSS variables (`--t`, `--h`) and classes
  (`rest-small open-medium`), so most size changes are pure CSS:
  e.g. task icons `calc(18px + 18px * var(--t))` grow smoothly as the dock opens.
* Widgets that change *structure* (tasks showing titles only at medium) use `onSize`.
* At **tiny**, content is hidden; only the shape shows.

## 3. Geometry & rendering

* **Bottom edge** = the same path flipped vertically (`scaleY(-1)` on the shape and
  on panel placement); widget content is not flipped. Panels open **upward**.
* **Taller notches** scale the profile from the reference SVG with height, so a
  60 px notch has the same silhouette as a 30 px one (flares/round-offs × height/30).
* **Dock mode**: the "expand" spring drives notch height (rest → open) and the
  notch width follows content measured at the open size, instead of spreading into
  a bar. Merged side notches, hugging and combine-overlapping work as today.
* **One window per surface per monitor** (`overlay-DISPLAY3-top`,
  `overlay-DISPLAY3-bottom`), each only as tall as it needs. Rejected alternative:
  one full-height window per monitor — fewer webviews, but a transparent, topmost
  full-screen window is expensive to composite and one region bug would block the
  whole screen. Cost of the chosen way: ~6 webviews on 3 monitors instead of 3
  (they share one WebView2 browser process).

## 4. Engine changes (Rust)

* `Overlay` keyed by `(monitor, surface)`; config resolves per surface
  (`Config::resolve_surface(rule, surface)` replaces `resolve_layout`).
* Hover, trigger zones and hit-testing take the surface's edge into account
  (`at_edge` = bottom edge for the dock). Hot corners stay a top-bar feature for now
  (bottom corners could be added the same way later).
* Window docking / "move aside for windows" and `hide_when_maximized` work per
  surface. For the dock this answers "a maximized window is under the dock":
  `shrink_when_covered = true` (default) shrinks it to its 12 px handle until
  hovered, like macOS intellihide; `false` keeps it at its rest size.
* Fullscreen hiding, DPI, monitor hot-plug: unchanged, applied to every surface.
* OSD grows the notch holding `osd.notch` on whichever surface it is.

## 5. Config & migration

* New: `[sizes]`, `[surfaces.<id>]`. `[notch]` side settings move into the surface
  (`[surfaces.top.notch.left]`…) with the current values as defaults.
* Per-monitor rules gain per-surface overrides:
  `[[monitors]] match = "ARZOPA"  surfaces = { bottom = { enabled = false } }`.
* **Migration v3 → v4** (settings window, like previous ones): `[layout]` +
  `[notch]` → `[surfaces.top]`; monitor `layout = {…}` → `surfaces.top = {…}`; add
  the default dock. Configs without `[surfaces]` keep working (read as top-only),
  so old presets still apply — they're migrated when loaded.
* `default-config.toml` gains the dock (your Default preset + the dock defaults).

## 6. Settings UI

* **Widgets page**: a surface switcher above Placement (`Status bar` · `Dock`, plus
  the existing Global / per-monitor switch). Drag & drop moves widgets between
  sides *and* surfaces (a widget lives in one place; see open questions).
* **New "Bars" section** per surface: enabled, edge, mode (Status bar / Dock),
  rest size, open size, shrink when covered, notch settings (merge, hug, offsets).
* Per-widget sections gain the new widgets' options (below).

## 7. New widgets

### Tasks — open windows
* **Data** (new provider `providers/tasks.rs`): Alt+Tab-style window list —
  visible, unowned, not tool windows, not cloaked (other virtual desktops / hidden
  UWP frames) — grouped by app (AppUserModelID, else exe). Event-driven with
  `SetWinEventHook` (create/destroy/show/hide/foreground/name change), no polling.
  Explorer folder windows show as "File Explorer — <folder>".
* **Icons**: window icon (`WM_GETICON`/class icon) → exe icon via the shell image
  factory; UWP apps via their package logo. Sent as small PNG data URLs, cached by app.
* **Actions**: click = activate (or minimize if already active); middle-click =
  close; right-click = small menu (close, close all, open new window).
  Activation from a no-focus overlay needs the existing `allow_foreground` +
  foreground-lock workaround.
* **Sizes**: small = 18 px icons, active app marked; medium = 36 px icons with a
  dot per window, title of the active one optional.
* **Which windows**: `monitor = "own"` (default — each surface shows the windows on
  its monitor) or `"all"`.
* Later (not in scope now): pinned apps, live thumbnails (DWM thumbnails), flashing
  for attention (shell hook).

### Quick settings
* **Icon** (`icon = "auto" | "battery" | "settings" | "volume" | "network"`):
  `auto` = battery level when the PC has a battery (`GetSystemPowerStatus`), else a
  settings gear.
* **Panel**: volume slider (Core Audio, already used by the OSD), brightness slider
  (WMI, laptops), battery % / time left / charging, network name & status, and
  buttons that open Windows' own pages (`ms-settings:` Wi-Fi, Bluetooth, display,
  sound…). Toggles for Wi-Fi/Bluetooth via the public WinRT Radio API can come
  later — nothing that needs private APIs.

### Launcher
* An **optional feature** (`[launcher] enabled`), separate from the Windows Start
  menu; the Win key stays untouched. Opened with a **hotkey** (default
  `Win+Shift+Space`, configurable — `Alt+Space` would clash with PowerToys Run and
  the window menu) and, optionally, the launcher widget.
* **Data**: every installed app from the Apps folder (`shell:AppsFolder`, which
  covers desktop shortcuts and Store apps alike), names + icons, cached and
  refreshed when Start menu folders change.
* **With the launcher widget** (a search icon, optional): clicking it — or the
  hotkey on that monitor — **morphs the widget into a text field** and an
  **attached results window** drops out of the bar/dock below (or above) it.
* **Without the widget**: the hotkey opens the attached window from the middle of
  the bar/dock on the monitor under the cursor, with its own search field on top.
* Results as you type; ↑/↓ to pick, Enter launches, Esc closes. Pinned apps shown
  while the field is empty.
* **Keyboard focus**: overlays are deliberately non-activating; while the launcher
  is open its window is allowed to take focus (the style guard gets an exception),
  and returns to non-activating on close.

### Buttons — custom icon buttons (multiple instances)
* A row of icon buttons, each with the same actions as menu items (`open` a path,
  app or `shell:` folder, `settings`, `sleep`, `lock`, `restart`, `shutdown`…),
  a tooltip, and optional `confirm` (second click).
* **Instances**: `buttons`, `buttons#2`, `buttons#3`… each placed independently with
  its own `[widgets."buttons#2"]` options — e.g. a power button in the top bar and a
  settings button in the dock. Settings gets "Add another" / "Remove".
* Instances are generic in the model (`<type>#<n>` → widget type `<type>`), so any
  widget can get them later; for now the UI offers it for Buttons.

## 8. Hiding the Windows taskbar

* Option `[shell] hide_taskbar` (dock settings): hides Explorer's taskbar while
  StatusNotch runs — **not removed or replaced**; Win key and Start keep working.
* How: set the taskbar to auto-hide through the official AppBar API (frees the
  screen area) **and** hide its window, so it doesn't pop up when the dock is hovered.
  Only Explorer's real `Shell_TrayWnd` (by process) is touched — never our own tray host.
* **Always restored**: on quit, on panic, on the next start after a crash
  (a marker file remembers the original state), and re-applied when Explorer
  restarts (`TaskbarCreated`). Secondary-monitor taskbars handled the same way.
* Interacts with the tray host and with Windhawk taskbar mods — needs testing on
  your setup first.

## 9. Phases

Each phase ends with a build you install and use; nothing half-done ships.

| # | Phase | Result | Risk |
|---|---|---|---|
| 1 | **Surfaces refactor** | Config v4, widget instances (`type#n`), per-surface windows & engine, bottom edge rendering. Top bar identical to v0.1.0; an optional bottom surface in bar mode works mirrored | High (touches everything) — tested against v0.1.0 side by side |
| 2 | **Dock mode & sizes** | `mode = dock`, tiny/small/medium morph, widget size context, `when_covered`, settings UI for surfaces | Medium |
| 3 | **Tasks widget** | Window tracker provider, icons, activate/minimize/close, small & medium looks | Medium (window filtering edge cases, focus rules) |
| 4 | **Hide Windows taskbar** | Option with crash-safe restore | Medium (Explorer / Windhawk interplay) |
| 5 | **Quick settings & Buttons** | Icon auto-select, panel with volume/brightness/battery/network/shortcuts; custom button instances | Low |
| 6 | **Launcher** | Hotkey, app index, search, pinned, widget→text field morph + attached results, focus handling | Medium (keyboard focus) |
| 7 | Defaults, installer 0.2.0, docs | Default preset with dock, ARCHITECTURE.md updated | Low |

Phases 3–6 are independent of each other after phase 2 and can be reordered.

## 10. Decisions (2026-10-08)

1. Hide the Windows taskbar by default when the dock is on — **yes**, restored on quit.
2. Dock under a maximized window — **shrinks to the 12 px handle** by default;
   setting to keep it at rest size.
3. Same widget in two places — not for built-in widgets yet; **instances exist in the
   model** and the Buttons widget uses them.
4. Launcher — **optional feature with a default hotkey**; the launcher widget is
   optional (widget → text field + attached results; no widget → hotkey opens the
   attached window with its own search field).
5. Tasks — **per monitor by default**, setting for all monitors.
6. New: **Buttons** widget with multiple instances.
