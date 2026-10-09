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
import { clipboard, clipboardPanel, focus, netspeed, notifications, notificationsPanel, privacy, privacyPanel, recycle } from "./extras";
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
  recycle,
  privacy,
  claude,
  clipboard,
  notifications,
  language: () => {
    // Click to switch the window you're typing in to the next keyboard layout.
    const root = el("widget language clickable");
    root.addEventListener("click", () => invoke("switch_keyboard_layout").catch(console.error));
    const unsubscribe = subscribe("keyboard", (k) => {
      root.textContent = k?.code ?? "";
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
