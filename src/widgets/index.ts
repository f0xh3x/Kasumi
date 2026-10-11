import { clock } from "./clock";
import { calendar } from "./calendar";
import { menu } from "./menu";
import { media } from "./media";
import { temps } from "./temps";
import { weather, weatherPanel } from "./weather";
import { mediaPanel } from "./media-panel";
import { tempsPanel } from "./temps-panel";
import { invoke } from "@tauri-apps/api/core";
import { trayButton, trayPanel } from "./tray";
import { tasks, tasksPanel } from "./tasks";
import { quick, quickPanel } from "./quick";
import { buttons } from "./buttons";
import { launcher, launcherPanel } from "./launcher";
import { clipboard, clipboardPanel, focus, netspeed, notifications, notificationsPanel, privacy, privacyPanel } from "./extras";
import { disk } from "./disk";
import { claude, claudePanel } from "./claude";
import { subscribe } from "../data";

export interface Widget {
  el: HTMLElement;
  dispose?: () => void;
  /** How its panel opens: a click on the widget (default), or only when the widget
   * dispatches a bubbling "notch:panel" event (e.g. on right click). */
  panelTrigger?: "click" | "event";
}

/** Lets a panel act on its overlay, e.g. close itself after an action. */
export interface PanelContext {
  close: () => void;
  /** The panel's content changed size; re-measure its dropdown. */
  resize: () => void;
}

type Options = Record<string, unknown>;
type Factory = (opts: Options) => Widget;
type PanelFactory = (opts: Options, ctx: PanelContext) => Widget;

/**
 * A Segoe Fluent Icons glyph by its codepoint. A function declaration, not a
 * const: widget modules this file imports call it while they load, before a
 * const here would exist (that stopped the whole page once).
 */
export function g(code: number): string {
  return String.fromCharCode(code);
}

/** Segoe Fluent Icons glyphs, the same codepoints as the YASB config. */
export const Glyph = {
  menu: g(0xe80f),
  prev: g(0xe892),
  next: g(0xe893),
  play: g(0xe768),
  pause: g(0xe769),
  temp: g(0xe9ca),
  sunny: g(0xe706),
  clearNight: g(0xec46),
} as const;

export function icon(glyph: string): HTMLSpanElement {
  const s = document.createElement("span");
  s.className = "icon";
  s.textContent = glyph;
  return s;
}

export function el(className: string, ...children: (Node | string)[]): HTMLElement {
  const e = document.createElement("div");
  e.className = className;
  e.append(...children);
  return e;
}

const widgets: Record<string, Factory> = {
  clock,
  temps,
  media,
  weather,
  menu: () => ({ el: el("widget menu", icon(Glyph.menu)) }),
  tray: trayButton,
  tasks,
  quick,
  buttons,
  launcher,
  focus,
  netspeed,
  disk,
  privacy,
  claude,
  clipboard,
  notifications,
  language: (opts) => {
    // Click to switch the window you're typing in to the next keyboard layout.
    // Options: icon (a keyboard glyph before the code), format "three" (ENG) | "two" (EN).
    const text = el("language-code");
    const root = el("widget language clickable", ...(opts.icon === true ? [icon(g(0xe765))] : []), text);
    root.addEventListener("click", () => invoke("switch_keyboard_layout").catch(console.error));
    const unsubscribe = subscribe("keyboard", (k) => {
      text.textContent = k ? languageCode(k, opts) : "";
      root.title = k?.name ?? "";
    });
    return { el: root, dispose: unsubscribe };
  },
};
const panels: Record<string, PanelFactory> = {
  calendar,
  menu,
  tray: trayPanel,
  media: mediaPanel,
  weather: weatherPanel,
  temps: tempsPanel,
  tasks: tasksPanel,
  quick: quickPanel,
  launcher: launcherPanel,
  privacy: privacyPanel,
  claude: claudePanel,
  clipboard: clipboardPanel,
  notifications: notificationsPanel,
};

/**
 * A placed widget's type: instances are named `<type>#<n>` (e.g. `buttons#2`),
 * each with its own `[widgets."buttons#2"]` options.
 */
export const widgetType = (name: string): string => name.split("#")[0];

/** A keyboard layout's code as the language widget shows it ([widgets.language] format). */
export function languageCode(k: { code: string; code2?: string }, opts: Options | undefined): string {
  return opts?.format === "two" && k.code2 ? k.code2 : k.code;
}

export function createWidget(name: string, opts: Options = {}): Widget {
  const factory = widgets[widgetType(name)];
  return factory ? factory(opts) : { el: el("widget unknown", name) };
}

export function createPanel(name: string, opts: Options, ctx: PanelContext): Widget {
  const factory = panels[widgetType(name)];
  return factory ? factory(opts, ctx) : { el: el("widget unknown", name) };
}

/** The panel a widget opens, wherever it's placed (bar dropdown or notch panel). */
export function panelFor(name: string): string | null {
  const type = widgetType(name);
  if (type === "clock") return name.replace(/^clock/, "calendar");
  return type in panels ? name : null;
}

/**
 * Sets an element's text. With `roll` ([animation] rolling), digits that change
 * roll to their new value like an odometer: up when the number grows, down when
 * it shrinks. Text of another length just switches.
 */
export function setText(el: HTMLElement, text: string, roll = false): void {
  if (el.dataset.text === text) return;
  const prev = el.dataset.text ?? el.textContent ?? "";
  el.dataset.text = text;
  if (!roll || prev.length !== text.length || prev === text) {
    el.textContent = text;
    return;
  }
  const num = (s: string) => Number(s.replace(/\D/g, "")) || 0;
  const up = num(text) >= num(prev);
  el.classList.add("rolling");
  const parts: Node[] = [];
  for (let i = 0; i < text.length; i++) {
    const a = prev[i];
    const b = text[i];
    if (a === b || !/\d/.test(a) || !/\d/.test(b)) {
      parts.push(document.createTextNode(b));
      continue;
    }
    const from = document.createElement("span");
    from.textContent = a;
    const to = document.createElement("span");
    to.textContent = b;
    const col = document.createElement("span");
    col.className = up ? "roll-col" : "roll-col down";
    if (up) col.append(from, to);
    else col.append(to, from);
    const box = document.createElement("span");
    box.className = "roll";
    box.append(col);
    parts.push(box);
  }
  el.replaceChildren(...parts);
  window.setTimeout(() => {
    if (el.dataset.text === text) el.textContent = text;
  }, 420);
}

/** An icon from Rust ({ w, h, rgba: base64 RGBA }) as a PNG data URL, cached per pixel payload. */
const urls = new Map<string, string>();
export function iconUrl(i: { w: number; h: number; rgba: string }): string {
  const cached = urls.get(i.rgba);
  if (cached) return cached;
  const bytes = Uint8ClampedArray.from(atob(i.rgba), (c) => c.charCodeAt(0));
  const canvas = document.createElement("canvas");
  canvas.width = i.w;
  canvas.height = i.h;
  canvas.getContext("2d")!.putImageData(new ImageData(bytes, i.w, i.h), 0, 0);
  const url = canvas.toDataURL();
  if (urls.size > 200) urls.clear();
  urls.set(i.rgba, url);
  return url;
}

/**
 * Runs `run` on click; with `confirm`, only on a second click within 3 s (the
 * first one arms it: `armed(true)`, and `armed(false)` when it runs or times
 * out). For destructive actions (restart, shut down). Returns a dispose.
 */
export function confirmClick(target: HTMLElement, confirm: boolean, run: () => void, armed: (on: boolean) => void = () => {}): () => void {
  let timer = 0;
  const disarm = () => {
    clearTimeout(timer);
    timer = 0;
    target.classList.remove("armed");
    armed(false);
  };
  target.addEventListener("click", (e) => {
    e.stopPropagation();
    if (confirm && !timer) {
      target.classList.add("armed");
      armed(true);
      timer = window.setTimeout(disarm, 3000);
      return;
    }
    disarm();
    run();
  });
  return () => clearTimeout(timer);
}

/** A small bar or ring showing a share (0..1): Claude usage, disk usage. */
export function meter(kind: "bar" | "circle", length: number) {
  if (kind === "circle") {
    const ns = "http://www.w3.org/2000/svg";
    const svg = document.createElementNS(ns, "svg");
    svg.setAttribute("viewBox", "0 0 16 16");
    svg.classList.add("meter-ring");
    const track = document.createElementNS(ns, "circle");
    const fill = document.createElementNS(ns, "circle");
    for (const c of [track, fill]) {
      c.setAttribute("cx", "8");
      c.setAttribute("cy", "8");
      c.setAttribute("r", "6");
    }
    track.classList.add("track");
    fill.classList.add("fill");
    svg.append(track, fill);
    return {
      el: svg as unknown as HTMLElement,
      set: (v: number) => svg.style.setProperty("--v", String(Math.min(1, Math.max(0, v)))),
    };
  }
  const fill = el("meter-bar-fill");
  const bar = el("meter-bar", fill);
  bar.style.setProperty("--len", `${length}px`);
  return { el: bar, set: (v: number) => bar.style.setProperty("--v", String(Math.min(1, Math.max(0, v)))) };
}

/** Bytes as Explorer shows them (1024-based): "931 GB", "1.8 TB", "512 MB". */
export function formatBytes(bytes: number): string {
  const units = ["B", "KB", "MB", "GB", "TB"];
  let v = bytes;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v >= 100 || i === 0 ? Math.round(v) : v.toFixed(1)} ${units[i]}`;
}

/** Apps' icons by AppUserModelID (or path, as shell:AppsFolder knows them), as
 *  PNG data URLs; null when there's none. Asked for once each. */
const appIcons = new Map<string, Promise<string | null>>();
const appIconsDone = new Map<string, string | null>();
export function appIcon(id: string): Promise<string | null> {
  let p = appIcons.get(id);
  if (!p) {
    p = invoke<{ w: number; h: number; rgba: string }>("launcher_icon", { id })
      .then((i) => iconUrl(i))
      .catch(() => null)
      .then((url) => (appIconsDone.set(id, url), url));
    appIcons.set(id, p);
  }
  return p;
}
/** The icon if it's already there (undefined: not asked for or not back yet). */
export const appIconNow = (id: string): string | null | undefined => appIconsDone.get(id);
