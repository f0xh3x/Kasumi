<p align="center">
  <img src="src-tauri/icons/kasumi.svg" width="96" alt="Kasumi logo">
</p>

<h1 align="center">Kasumi</h1>

<p align="center">A notch-style taskbar and Start menu replacement for Windows.</p>

> [!NOTE]
> This software was entirely vibecoded. I have no programming skills and just wanted a
> more pleasant Windows experience.

Kasumi puts small black notches at the top and bottom of your screens. Hover one and it
opens into a status bar, a dock with your apps, or a panel: a calendar, the weather,
media controls, quick settings and more. The Windows taskbar steps out of the way while
Kasumi runs and comes back when it quits.

<!-- Screenshots / a short GIF go here. -->

## Features

- **Notches that open into a bar**, at the top and bottom of every monitor, each with
  its own layout if you like. They step aside for a maximized window's buttons and hide
  over fullscreen games and videos.
- **A dock** with your running and pinned apps, and a launcher that stands in for the
  Start menu (Windows key included).
- **Widgets**: clock and calendar, weather, media, PC temperatures, quick settings
  (volume, brightness, audio outputs), system tray, notifications, clipboard history,
  network speed, disk usage, privacy indicators, keyboard layout, Komorebi workspaces
  and your own buttons.
- **Pop-ups** for volume, brightness, Caps Lock, keyboard layout and Windows
  notifications, each placed where you want it: which screen, which notch.
- **Make it yours**: colours, fonts (any installed font, separately for main and
  secondary text), the Windows accent colour, and animations with per-widget options.
- **Presets** to switch between whole setups in one click.

## Install

Download the installer from [Releases](../../releases) and run it. Kasumi starts with
Windows unless you turn that off in its settings (open them from the tray icon).

The installer isn't code-signed, so Windows SmartScreen may warn the first time:
choose **More info → Run anyway**.

Requirements: Windows 11 (Windows 10 may work) with WebView2, which Windows 11
already has.

### Tips

- **Notifications:** Kasumi reads Windows' own notifications, so keep them on in
  Windows and turn on *Do not disturb* to hide Windows' pop-ups (Settings › OSD ›
  Notifications walks you through it).
- **Komorebi:** the Workspaces widget shows and switches
  [Komorebi](https://github.com/LGUG2Z/komorebi) workspaces while it runs.

## Building from source

You need [Node.js](https://nodejs.org), [Rust](https://rustup.rs) and the
[Tauri prerequisites](https://tauri.app/start/prerequisites/) for Windows.

```sh
npm install
npx tauri build          # the installer lands in src-tauri/target/release/bundle/nsis/
```

`ARCHITECTURE.md` explains how it fits together; `CLAUDE.md` holds the rules and
helpers used while building it.

## License

Public domain ([The Unlicense](LICENSE)): use it however you like, no credit needed.
