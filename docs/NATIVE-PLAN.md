# Native renderer: plan

Goal: the always-running bars and docks drawn natively (Direct2D + DirectWrite on
layered windows, no WebView2), so Kasumi runs light on older PCs. Settings, first-run
setup and the update notes stay WebView2 windows (opened rarely, closed after).
The prototype in `native-test/` proved the look, motion and cost (~16 MB vs ~375 MB).

## Shape of it

* **Inside Kasumi**, not a second app: the Rust backend stays as it is (config,
  engine hover / docking / fullscreen, providers, OSD, launcher index, tray host,
  taskbar hiding, updater). Only the overlay pages are replaced.
* **Switch:** `[general] renderer = "web" | "native"` (default `web` until the
  native bars cover everything, then `native`). Read at start.
* **Seam:** `ui.rs`. Every place the engine and providers talked to a page
  (`emit_to`, `get_webview_window`, `set_ignore_cursor_events`, `win::place`) goes
  through it: web overlays get Tauri events as before, native surfaces get typed
  messages on the native thread.
* **Native thread** (`native/`): one thread, its own message loop, one layered
  window per surface. `Surface` is the port of `Overlay` (src/main.ts): drops,
  springs, the one shape (`native/geometry.rs` = src/geometry.ts), content drawn
  over it and clipped by it. It reports what the page reported (notch widths,
  regions, content height) by calling the engine directly.
* **Widgets** are Rust types (`native/widgets/`): measure, draw, hit, click, data
  keys they use, optional panel. They call the backend's functions directly
  (what the pages `invoke`d).
* **Testing without the screen:** `kasumi.exe --native-snapshot <dir>` builds the
  layouts from the live config and the real monitors, draws each surface in its
  states into BMPs off screen and exits. It doesn't claim the single instance, so
  the running Kasumi is left alone.

## Phases

1. **Foundation.** Renderer switch, `ui.rs` seam, native thread + surfaces: the
   shape (bar and dock modes, top and bottom edges, open / close, spread, notch
   slides, retract, nudge, rounded corners), notch measuring and regions back to
   the engine, Inter bundled (DirectWrite in-memory font), the snapshot mode.
   Widgets: clock (+ calendar), menu.
2. **Text widgets:** language, buttons, temps, weather, disk, focus, netspeed,
   privacy, claude, workspaces; widget fonts / accent / dim; the open bar's
   layout (groups, center split, pinned widgets gliding from their notch).
3. **Icons and lists:** tasks (from the prototype + providers/tasks), tray, media
   (keys, title, art, marquee).
4. **Panels:** calendar (done in 1), weather, temps, media, quick settings
   (sliders, outputs, per-app volume, tiles), tray overflow, tasks menu, clipboard,
   notifications, claude, privacy. Cascade, panel content styles.
5. **Pop-ups and input:** OSD pill, notifications out of the notch, keyboard peek,
   launcher (text field, results, keyboard), files dropped on the bar (OLE drop
   target), tooltips.
6. **Switch over:** parity pass against the web bars, measure memory / CPU, make
   `native` the default, a settings toggle, release notes.

Each phase is committed on branch `native`; the web renderer keeps working the
whole time.

## Status (2026-10-22)

Phases 1-5 done (tooltips and the covered / slide motions included), shipped as
"Native Kasumi (experimental)" in 0.5.0 (Settings > About). Native-only extras
since: tasks attention dots, window previews, Win + 1..9, scroll on apps, files
dragged onto apps, power tiles in quick settings. Next: the user tries it daily,
fix what shows, then make native the default.
