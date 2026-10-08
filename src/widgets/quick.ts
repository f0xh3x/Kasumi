import { invoke } from "@tauri-apps/api/core";
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

const SHORTCUTS: [string, number, string][] = [
  ["Wi-Fi", 0xe701, "ms-settings:network-wifi"],
  ["Bluetooth", 0xe702, "ms-settings:bluetooth"],
  ["Display", 0xe7f4, "ms-settings:display"],
  ["Sound", 0xe767, "ms-settings:sound"],
  ["Power", 0xe7e8, "ms-settings:powersleep"],
  ["Settings", 0xe713, "ms-settings:"],
];

export function quickPanel(_opts: Record<string, unknown>, ctx: PanelContext): Widget {
  const volume = slider(
    (v) => invoke("volume_set", { level: v / 100 }).catch(console.error),
    () => invoke("volume_toggle_mute").catch(console.error),
  );
  const brightness = slider((v) => invoke("brightness_set", { level: v }).catch(console.error));
  const battery = el("quick-status");
  const network = el("quick-status");
  const tiles = el(
    "quick-tiles",
    ...SHORTCUTS.map(([label, glyph, target]) => {
      const t = el("quick-tile", icon(g(glyph)), el("quick-tile-label", label));
      t.addEventListener("click", () => {
        invoke("run_action", { action: "open", target }).catch(console.error);
        ctx.close();
      });
      return t;
    }),
  );
  const root = el("quick-panel", volume.row, brightness.row, battery, network, tiles);
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
  return { el: root, dispose: off };
}
