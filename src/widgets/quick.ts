import { invoke } from "@tauri-apps/api/core";
import { DEFAULT_TILES, QUICK_TILES } from "../quick-tiles";
import { subscribe, type SystemStatus } from "../data";
import { el, icon, type PanelContext, type Widget } from "./index";

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
 * Slides a dropdown list open or shut (height and fade); the panel's shape follows
 * every frame. `done` runs when it's settled.
 */
function slide(list: HTMLElement, open: boolean, ctx: PanelContext): void {
  const from = list.hidden ? 0 : list.offsetHeight;
  list.hidden = false;
  list.style.height = "";
  const to = open ? list.scrollHeight : 0;
  list.style.overflow = "hidden";
  // Start where it is (no flash of the full list before the first frame).
  list.style.height = `${from}px`;
  list.style.opacity = open ? "0" : "1";
  const start = performance.now();
  const step = (now: number) => {
    const k = Math.min(1, (now - start) / 180);
    const e = 1 - (1 - k) ** 3;
    list.style.height = `${from + (to - from) * e}px`;
    list.style.opacity = String(open ? e : 1 - e);
    ctx.resize();
    if (k < 1) {
      requestAnimationFrame(step);
      return;
    }
    list.style.height = list.style.overflow = list.style.opacity = "";
    list.hidden = !open;
    ctx.resize();
  };
  requestAnimationFrame(step);
}

/** Output devices: the list to switch to, opened from an icon in the volume row. */
function outputPicker(ctx: PanelContext) {
  const list = el("quick-drop quick-output-list");
  list.hidden = true;
  const button = icon(g(0xe7f5));
  button.classList.add("quick-expand", "clickable");
  let devices: { id: string; name: string; default: boolean }[] = [];
  const state = { open: false };
  const draw = () => {
    button.title = `Output: ${devices.find((d) => d.default)?.name ?? "none"}`;
    list.replaceChildren(
      ...devices.map((d) => {
        const row = el(`quick-output-item${d.default ? " on" : ""}`, icon(g(d.default ? 0xe73e : 0xe7f5)), el("", d.name));
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
        if (state.open) ctx.resize();
      })
      .catch(console.error);
  const toggle = (open = !state.open) => {
    if (open === state.open) return;
    state.open = open;
    button.classList.toggle("open", open);
    if (open) refresh();
    slide(list, open, ctx);
  };
  return { list, button, refresh, toggle, state };
}

/** Per-app volume: a slider per app playing on the default output, in a list
 *  that opens from the volume row's chevron (closed by default). */
function appVolumes(ctx: PanelContext) {
  const list = el("quick-drop quick-apps");
  const empty = el("quick-apps-empty", "No apps are playing sound.");
  list.append(empty);
  list.hidden = true;
  const state = { open: false };
  const rows = new Map<number, { row: HTMLElement; input: HTMLInputElement; mute: HTMLElement; dragging: boolean }>();
  const refresh = () =>
    invoke<{ pid: number; name: string; level: number; muted: boolean }[]>("audio_sessions")
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
            const mute = icon("");
            mute.classList.add("clickable");
            const row = el("quick-app", el("quick-app-name", a.name), input, mute);
            row.title = a.name;
            const entry = { row, input, mute, dragging: false };
            input.addEventListener("input", () => {
              entry.dragging = true;
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
            r.input.value = String(Math.round(a.level * 100));
            r.input.style.setProperty("--fill", `${a.muted ? 0 : a.level * 100}%`);
          }
          r.mute.classList.toggle("muted", a.muted);
          r.mute.textContent = g(a.muted ? 0xe74f : 0xe767);
        }
        empty.hidden = rows.size > 0;
        if (changed && state.open) ctx.resize();
      })
      .catch(console.error);
  const chevron = icon(g(0xe70d));
  chevron.classList.add("quick-expand", "chevron", "clickable");
  chevron.title = "Apps";
  const toggle = (open = !state.open) => {
    if (open === state.open) return;
    state.open = open;
    chevron.classList.toggle("open", open);
    if (open) refresh();
    slide(list, open, ctx);
  };
  return { list, button: chevron, refresh, toggle, state };
}

export function quickPanel(opts: Record<string, unknown>, ctx: PanelContext): Widget {
  const volume = slider(
    (v) => invoke("volume_set", { level: v / 100 }).catch(console.error),
    () => invoke("volume_toggle_mute").catch(console.error),
  );
  const brightness = slider((v) => invoke("brightness_set", { level: v }).catch(console.error));
  const battery = el("quick-status");
  const network = el("quick-status");
  const chosen = Array.isArray(opts.tiles) ? (opts.tiles as unknown[]).filter((t): t is string => typeof t === "string" && t in QUICK_TILES) : DEFAULT_TILES;
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
  tiles.hidden = chosen.length === 0;
  const outputs = opts.outputs === true ? outputPicker(ctx) : null;
  const apps = opts.apps === true ? appVolumes(ctx) : null;
  // Icons at the end of the volume row open their lists, one at a time. The lists
  // open toward the screen edge (CSS: above the row on a bottom bar), so the row
  // stays under the cursor while the panel grows and shrinks.
  const drops = [outputs, apps].filter((d) => d !== null);
  for (const d of drops) {
    d.button.addEventListener("click", (e) => {
      e.stopPropagation();
      const open = !d.state.open;
      for (const o of drops) if (o !== d) o.toggle(false);
      d.toggle(open);
    });
    volume.row.append(d.button);
  }
  const root = el(
    "quick-panel",
    volume.row,
    ...drops.map((d) => d.list),
    brightness.row,
    battery,
    network,
    tiles,
  );
  // Devices and apps are asked for only while the panel shows.
  const shown = () => {
    const p = root.closest<HTMLElement>(".panel");
    return !!p && p.style.visibility !== "hidden";
  };
  let tick = 0;
  const poll = window.setInterval(() => {
    if (!shown()) {
      tick = 0;
      return;
    }
    if (tick % 3 === 0) outputs?.refresh();
    if (apps?.state.open) apps.refresh();
    tick++;
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
    // Rows appearing or going away change the panel's size.
    const now = `${volume.row.hidden}${brightness.row.hidden}${battery.hidden}`;
    if (now !== shape) {
      shape = now;
      ctx.resize();
    }
  });
  return { el: root, dispose: () => (off(), clearInterval(poll)) };
}
