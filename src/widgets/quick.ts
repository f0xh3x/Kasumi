import { invoke } from "@tauri-apps/api/core";
import { DEFAULT_TILES, MAX_TILES, QUICK_TILES } from "../quick-tiles";
import { subscribe, type SystemStatus } from "../data";
import { el, icon, type PanelContext, type Widget } from "./index";
import { iconUrl } from "./tray";

/**
 * Quick settings. The button shows the battery (on PCs that have one) or a
 * settings gear (`icon` = "auto" | "battery" | "settings" | "volume" | "network");
 * its panel has volume and brightness sliders, battery and network status, and
 * shortcuts to Windows' own settings pages.
 */

const g = (c: number) => String.fromCharCode(c);

export function batteryGlyph(percent: number, charging: boolean): string {
  const step = Math.max(0, Math.min(10, Math.round(percent / 10)));
  if (charging) return step >= 10 ? g(0xea93) : step === 9 ? g(0xe83e) : g(0xe85a + step);
  return step >= 10 ? g(0xe83f) : g(0xe850 + step);
}

const volumeGlyph = (level: number, muted: boolean) =>
  muted ? g(0xe74f) : level === 0 ? g(0xe992) : level < 0.34 ? g(0xe993) : level < 0.67 ? g(0xe994) : g(0xe995);

const networkGlyph = (n: SystemStatus["network"]) => (!n?.connected ? g(0xf384) : n.wifi ? g(0xe701) : g(0xe839));

function formatLeft(seconds: number | null | undefined): string {
  if (!seconds) return "";
  const h = Math.floor(seconds / 3600);
  const m = Math.round((seconds % 3600) / 60);
  return h ? `${h} h ${m} min left` : `${m} min left`;
}

export function quick(opts: Record<string, unknown>): Widget {
  const mode = typeof opts.icon === "string" ? opts.icon : "auto";
  const glyph = icon(g(0xe713));
  const value = el("quick-value");
  const root = el("widget quick", glyph, value);
  const off = subscribe("system", (s) => {
    const battery = s?.battery ?? null;
    const show = mode === "auto" ? (battery ? "battery" : "settings") : mode;
    value.textContent = "";
    root.title = "Quick settings";
    if (show === "battery" && battery && battery.percent != null) {
      glyph.textContent = batteryGlyph(battery.percent, battery.charging);
      value.textContent = `${battery.percent}%`;
      root.title = `Battery ${battery.percent}%${battery.charging ? ", charging" : ""}`;
    } else if (show === "volume" && s?.volume) {
      glyph.textContent = volumeGlyph(s.volume.level, s.volume.muted);
    } else if (show === "network") {
      glyph.textContent = networkGlyph(s?.network ?? null);
      root.title = s?.network?.connected ? s.network.name : "Not connected";
    } else if (show === "custom" && typeof opts.custom_icon === "string" && opts.custom_icon) {
      glyph.textContent = opts.custom_icon;
    } else {
      glyph.textContent = g(0xe713);
    }
  });
  return { el: root, dispose: off };
}

/** A slider row: icon (click: `onIcon`), range input, value. */
function slider(onInput: (v: number) => void, onIcon?: () => void) {
  const ico = icon("");
  const input = document.createElement("input");
  input.type = "range";
  input.min = "0";
  input.max = "100";
  input.className = "quick-range";
  const val = el("quick-num");
  let timer = 0;
  let dragging = false;
  input.addEventListener("input", () => {
    dragging = true;
    val.textContent = input.value;
    // The filled part of the track follows the thumb right away.
    input.style.setProperty("--fill", `${input.value}%`);
    clearTimeout(timer);
    timer = window.setTimeout(() => onInput(Number(input.value)), 40);
  });
  let release = 0;
  input.addEventListener("change", () => {
    clearTimeout(timer);
    onInput(Number(input.value));
    // Let the change land before taking readings again.
    clearTimeout(release);
    release = window.setTimeout(() => (dragging = false), 400);
  });
  if (onIcon) {
    ico.classList.add("clickable");
    ico.addEventListener("click", onIcon);
  }
  const row = el("quick-row", ico, input, val);
  return {
    row,
    set(v: number, glyph: string) {
      ico.textContent = glyph;
      if (dragging) return;
      input.value = String(Math.round(v));
      val.textContent = String(Math.round(v));
      input.style.setProperty("--fill", `${v}%`);
    },
  };
}

/**
 * Slides `wrap` open or shut: its height (it has no padding, so it reaches 0 px),
 * its gap in the panel (a hidden element takes none) and a fade. Opening from
 * open (new content) animates between the two heights. The panel's shape
 * follows every frame.
 */
function slide(wrap: HTMLElement, open: boolean, ctx: PanelContext, done?: () => void): void {
  const gap = parseFloat(getComputedStyle(wrap.parentElement!).rowGap) || 0;
  const wasHidden = wrap.hidden;
  const from = wasHidden ? 0 : wrap.getBoundingClientRect().height;
  const gapFrom = wasHidden ? 0 : gap;
  wrap.hidden = false;
  wrap.style.height = "auto";
  const to = open ? wrap.getBoundingClientRect().height : 0;
  const gapTo = open ? gap : 0;
  const fade = wasHidden || !open;
  wrap.style.overflow = "hidden";
  const at = (e: number) => {
    wrap.style.height = `${from + (to - from) * e}px`;
    wrap.style.marginBottom = `${gapFrom + (gapTo - gapFrom) * e - gap}px`;
    if (fade) wrap.style.opacity = String(open ? e : 1 - e);
  };
  at(0);
  ctx.resize();
  const start = performance.now();
  const step = (now: number) => {
    const k = Math.min(1, (now - start) / 200);
    at(1 - (1 - k) ** 3);
    if (k < 1) {
      ctx.resize();
      requestAnimationFrame(step);
      return;
    }
    wrap.style.height = wrap.style.overflow = wrap.style.opacity = wrap.style.marginBottom = "";
    wrap.hidden = !open;
    ctx.resize();
    done?.();
  };
  requestAnimationFrame(step);
}

/** Output devices, two per row; a click makes one the default. */
function outputPicker(ctx: PanelContext) {
  const list = el("quick-outputs");
  const button = icon(g(0xe7f5));
  button.classList.add("quick-expand", "clickable");
  let devices: { id: string; name: string; default: boolean }[] = [];
  const draw = () => {
    button.title = `Output: ${devices.find((d) => d.default)?.name ?? "none"}`;
    list.replaceChildren(
      ...devices.map((d) => {
        const row = el(`quick-output-item${d.default ? " on" : ""}`, icon(g(d.default ? 0xe73e : 0xe7f5)), el("quick-output-name", d.name));
        row.title = d.name;
        row.addEventListener("click", (e) => {
          e.stopPropagation();
          invoke("audio_set_output", { id: d.id })
            .then(refresh)
            .catch(console.error);
        });
        return row;
      }),
    );
  };
  const refresh = () =>
    invoke<typeof devices>("audio_outputs")
      .then((d) => {
        if (JSON.stringify(d) === JSON.stringify(devices)) return;
        devices = d;
        draw();
        ctx.resize();
      })
      .catch(console.error);
  return { name: "outputs", list, button, refresh };
}

/** Per-app volume, two per row: the app's icon (click: mute), a slider, the value. */
function appVolumes(ctx: PanelContext) {
  const list = el("quick-apps");
  const empty = el("quick-apps-empty", "No apps are playing sound.");
  list.append(empty);
  type Row = { row: HTMLElement; input: HTMLInputElement; mute: HTMLElement; val: HTMLElement; dragging: boolean };
  const rows = new Map<number, Row>();
  type App = { pid: number; name: string; icon: { w: number; h: number; rgba: string } | null; level: number; muted: boolean };
  const refresh = () =>
    invoke<App[]>("audio_sessions")
      .then((apps) => {
        let changed = false;
        for (const [pid, r] of rows) {
          if (!apps.some((a) => a.pid === pid)) {
            r.row.remove();
            rows.delete(pid);
            changed = true;
          }
        }
        for (const a of apps) {
          let r = rows.get(a.pid);
          if (!r) {
            const input = document.createElement("input");
            input.type = "range";
            input.className = "quick-range";
            input.min = "0";
            input.max = "100";
            const mute = el("quick-app-icon clickable");
            if (a.icon) {
              const img = document.createElement("img");
              img.src = iconUrl(a.icon);
              img.draggable = false;
              mute.append(img);
            } else mute.append(icon(g(0xe767)));
            mute.title = `${a.name} — click to mute`;
            const val = el("quick-num");
            const row = el("quick-app", mute, input, val);
            const entry: Row = { row, input, mute, val, dragging: false };
            input.addEventListener("input", () => {
              entry.dragging = true;
              val.textContent = input.value;
              input.style.setProperty("--fill", `${input.value}%`);
              invoke("audio_session_set", { pid: a.pid, level: Number(input.value) / 100 }).catch(console.error);
            });
            input.addEventListener("change", () => window.setTimeout(() => (entry.dragging = false), 400));
            mute.addEventListener("click", (e) => {
              e.stopPropagation();
              invoke("audio_session_set", { pid: a.pid, muted: !mute.classList.contains("muted") }).then(refresh).catch(console.error);
            });
            rows.set(a.pid, (r = entry));
            list.append(row);
            changed = true;
          }
          if (!r.dragging) {
            const v = Math.round(a.level * 100);
            r.input.value = String(v);
            r.val.textContent = String(v);
            r.input.style.setProperty("--fill", `${a.muted ? 0 : v}%`);
          }
          r.mute.classList.toggle("muted", a.muted);
        }
        empty.hidden = rows.size > 0;
        if (changed) ctx.resize();
      })
      .catch(console.error);
  const chevron = icon(g(0xe70e));
  chevron.classList.add("quick-expand", "chevron", "clickable");
  chevron.title = "Apps";
  return { name: "apps", list, button: chevron, refresh };
}

/**
 * Quick settings panel: volume (with per-app volume and output devices behind
 * icons at its end) and brightness side by side, battery and network, then a
 * row of shortcut tiles. The per-app / output list opens in a box toward the
 * screen edge (above the controls on a bottom bar), so the controls stay put.
 */
export function quickPanel(opts: Record<string, unknown>, ctx: PanelContext): Widget {
  const volume = slider(
    (v) => invoke("volume_set", { level: v / 100 }).catch(console.error),
    () => invoke("volume_toggle_mute").catch(console.error),
  );
  const brightness = slider((v) => invoke("brightness_set", { level: v }).catch(console.error));
  const battery = el("quick-status");
  const network = el("quick-status");
  const chosen = (Array.isArray(opts.tiles) ? (opts.tiles as unknown[]).filter((t): t is string => typeof t === "string" && t in QUICK_TILES) : DEFAULT_TILES).slice(0, MAX_TILES);
  const tiles = el(
    "quick-tiles",
    ...chosen.map((id) => {
      const [label, glyph, target] = QUICK_TILES[id];
      const t = el("quick-tile", icon(g(glyph)), el("quick-tile-label", label));
      t.addEventListener("click", () => {
        invoke("run_action", { action: "open", target }).catch(console.error);
        ctx.close();
      });
      return t;
    }),
  );
  // One row filling the width; with three or fewer they're slim pills, not blocks.
  tiles.style.gridTemplateColumns = `repeat(${Math.max(1, chosen.length)}, minmax(0, 1fr))`;
  tiles.classList.toggle("pills", chosen.length <= 3);
  tiles.hidden = chosen.length === 0;
  const outputs = opts.outputs === true ? outputPicker(ctx) : null;
  const apps = opts.apps === true ? appVolumes(ctx) : null;

  // The box the lists open in (one at a time; the same icon again closes it).
  const box = el("quick-more-box");
  const more = el("quick-more", box);
  more.hidden = true;
  let current: string | null = null;
  const drops = [apps, outputs].filter((d) => d !== null);
  const close = () => {
    current = null;
    for (const d of drops) d.button.classList.remove("open");
    slide(more, false, ctx, () => current === null && box.replaceChildren());
  };
  for (const d of drops) {
    d.button.addEventListener("click", (e) => {
      e.stopPropagation();
      if (current === d.name) return close();
      current = d.name;
      for (const o of drops) o.button.classList.toggle("open", o === d);
      // Fetch first, so it opens at its real height.
      d.refresh().finally(() => {
        if (current !== d.name) return;
        box.replaceChildren(d.list);
        slide(more, true, ctx);
      });
    });
    volume.row.append(d.button);
  }

  const grid = el("quick-grid", volume.row, brightness.row, battery, network);
  const root = el("quick-panel", more, grid, tiles);
  // Devices and apps are asked for only while their list shows.
  const shown = () => {
    const p = root.closest<HTMLElement>(".panel");
    return !!p && p.style.visibility !== "hidden";
  };
  const poll = window.setInterval(() => {
    if (!shown()) {
      // The panel closed: the list goes with it (shut, no animation).
      if (current !== null) {
        current = null;
        for (const d of drops) d.button.classList.remove("open");
        more.hidden = true;
        box.replaceChildren();
      }
      return;
    }
    if (current === "outputs") outputs?.refresh();
    if (current === "apps") apps?.refresh();
  }, 1000);
  let shape = "";
  const off = subscribe("system", (s) => {
    volume.row.hidden = !s?.volume;
    if (s?.volume) volume.set(s.volume.muted ? 0 : s.volume.level * 100, volumeGlyph(s.volume.level, s.volume.muted));
    brightness.row.hidden = s?.brightness == null;
    if (s?.brightness != null) brightness.set(s.brightness, g(0xe706));
    const b = s?.battery;
    battery.hidden = !b;
    if (b) {
      const state = b.charging ? "Charging" : b.plugged ? "Plugged in" : formatLeft(b.secondsLeft);
      battery.replaceChildren(icon(batteryGlyph(b.percent ?? 0, b.charging)), el("", `${b.percent ?? "?"}%${state ? ` · ${state}` : ""}`));
    }
    const n = s?.network;
    network.replaceChildren(
      icon(networkGlyph(n ?? null)),
      el("", n?.connected ? `${n.name || (n.wifi ? "Wi-Fi" : "Ethernet")}${n.internet ? "" : " · No internet"}` : "Not connected"),
    );
    // A row without its neighbour (no brightness / no battery here) takes the whole width.
    volume.row.style.gridColumn = brightness.row.hidden ? "1 / -1" : "";
    brightness.row.style.gridColumn = volume.row.hidden ? "1 / -1" : "";
    network.style.gridColumn = battery.hidden ? "1 / -1" : "";
    // Rows appearing or going away change the panel's size.
    const now = `${volume.row.hidden}${brightness.row.hidden}${battery.hidden}`;
    if (now !== shape) {
      shape = now;
      ctx.resize();
    }
  });
  return { el: root, dispose: () => (off(), clearInterval(poll)) };
}
