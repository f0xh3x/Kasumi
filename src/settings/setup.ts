// The first-time setup and the update notes: views of the settings page in their
// own windows (settings.html?view=setup / ?view=notes, opened by settings.rs).
// Everything they change is saved at once, like in the settings window.
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { kasumiLogo } from "../quick-tiles";
import { $, glyph, toggleSwitch } from "./dom";
import { CHIP_LEGEND, placementHide, scope, type Op } from "./editors";
import { renderMarkdown } from "./markdown";
import type { Cfg, Field } from "./schema";

/** What the views use from the settings page (main.ts). */
export interface Host {
  cfg(): Cfg;
  /** A schema field as a settings row (saves on change). */
  control(f: Field): HTMLElement | null;
  saveMany(ops: Op[]): Promise<void>;
  /** Re-reads the config and redraws. */
  reload(): Promise<void>;
  render(): void;
  toast(text: string, error?: boolean): void;
}

// --- Pictures ----------------------------------------------------------------------

type Style = "bar" | "dock" | "always" | "off";

/** A notch hanging from y = 0, from x0 to x1, `d` deep. */
const notchPath = (x0: number, x1: number, d: number) =>
  `M${x0} 0C${x0 + 3} 0 ${x0 + 3} ${d} ${x0 + 7} ${d}L${x1 - 7} ${d}C${x1 - 3} ${d} ${x1 - 3} 0 ${x1} 0Z`;

/** One bar on a 160 × 100 screen, drawn from the top edge (the bottom one is flipped). */
function barShapes(style: Style, edge: "top" | "bottom"): string {
  if (style === "off") return "";
  const flip = edge === "bottom" ? ' transform="matrix(1 0 0 -1 0 100)"' : "";
  const marks = (xs: number[], y: number) => xs.map((x) => `<rect x="${x}" y="${y - 0.8}" width="7" height="1.6" rx="0.8" fill="#fff" opacity="0.75"/>`).join("");
  const icons = (xs: number[], y: number) => xs.map((x) => `<rect x="${x}" y="${y - 2.5}" width="5" height="5" rx="1.4" fill="#fff" opacity="0.8"/>`).join("");
  let body = "";
  if (style === "always") {
    body = `<rect x="0" y="0" width="160" height="7" fill="#000"/>${marks([5, 16], 3.5)}${marks([76], 3.5)}${marks([137, 148], 3.5)}`;
  } else if (style === "bar") {
    body =
      `<path d="${notchPath(-4, 34, 7)}" fill="#000"/><path d="${notchPath(60, 100, 7)}" fill="#000"/><path d="${notchPath(126, 164, 7)}" fill="#000"/>` +
      marks([4, 15], 3.3) + marks([76], 3.3) + marks([136, 147], 3.3);
  } else {
    body = `<path d="${notchPath(-4, 30, 9)}" fill="#000"/><path d="${notchPath(52, 108, 9)}" fill="#000"/>` + icons([4, 13, 22], 4.5) + icons([60, 69, 78, 87, 96], 4.5);
  }
  return `<g${flip}>${body}</g>`;
}

function screen(inner: string): HTMLElement {
  const box = $("div", "setup-screen");
  box.innerHTML =
    '<svg viewBox="0 0 160 100" aria-hidden="true"><defs><linearGradient id="wall" x1="0" y1="0" x2="1" y2="1">' +
    '<stop offset="0" stop-color="#7fb7c9"/><stop offset="0.55" stop-color="#4b7f99"/><stop offset="1" stop-color="#28485e"/></linearGradient></defs>' +
    `<rect width="160" height="100" fill="url(#wall)"/>${inner}</svg>`;
  return box;
}

/** A maximized window under the top bar: its caption buttons in the top-right corner,
 *  the corner notch over them or moved aside. */
function asideScreen(avoid: boolean): HTMLElement {
  const win =
    '<rect x="0" y="0" width="160" height="100" fill="#e9edf1"/><rect x="0" y="0" width="160" height="11" fill="#d7dde3"/>' +
    '<g stroke="#3a3f44" stroke-width="1" fill="none"><path d="M131 6.5h5"/><rect x="141" y="4" width="4.5" height="4.5"/><path d="M151 4l4.5 4.5M155.5 4l-4.5 4.5"/></g>' +
    '<rect x="8" y="20" width="62" height="5" rx="2.5" fill="#c3cad1"/><rect x="8" y="30" width="96" height="4" rx="2" fill="#d2d8de"/><rect x="8" y="38" width="80" height="4" rx="2" fill="#d2d8de"/>';
  const right = avoid ? notchPath(88, 126, 7) : notchPath(126, 164, 7);
  const marks = (x: number) => `<rect x="${x}" y="2.5" width="7" height="1.6" rx="0.8" fill="#fff" opacity="0.75"/>`;
  const arrow = avoid ? '<path d="M121 15l-6 0m2.5 -2.5l-2.5 2.5 2.5 2.5" stroke="#000" stroke-width="1.2" fill="none" stroke-linecap="round"/>' : "";
  return screen(`${win}<path d="${notchPath(-4, 34, 7)}" fill="#000"/>${marks(4)}${marks(15)}<path d="${right}" fill="#000"/>${marks(avoid ? 98 : 136)}${marks(avoid ? 109 : 147)}${arrow}`);
}

// --- Setup -------------------------------------------------------------------------

type Step = "welcome" | "presets" | "general" | "top" | "bottom" | "widgets" | "osd" | "done";
const QUICK: Step[] = ["general", "top", "bottom", "widgets", "osd"];
/** Widgets the setup leaves out of its placement: they need more setting up first. */
const SETUP_HIDDEN = ["claude", "workspaces", "temps"];

let step: Step = "welcome";
let flow: "quick" | "presets" = "quick";
let picked: string | null = null;
let shipped: { name: string; top: Style; bottom: Style }[] | null = null;

const STYLES: [Style, string, string][] = [
  ["bar", "Notches", "Small notches that open into a full bar when you go to the edge."],
  ["dock", "Dock", "Notches that grow in place, like a dock."],
  ["always", "Status bar", "A full bar, always shown. Windows make room for it."],
  ["off", "Off", "No bar on this edge."],
];

const styleOf = (cfg: Cfg, s: string): Style => {
  const o = cfg.surfaces?.[s];
  if (!o || o.enabled === false) return "off";
  return (o.mode ?? "bar") as Style;
};

function styleOps(s: string, style: Style): Op[] {
  if (style === "off") return [{ path: ["surfaces", s, "enabled"], value: false }];
  return [
    { path: ["surfaces", s, "enabled"], value: true },
    { path: ["surfaces", s, "mode"], value: style },
    // A status bar keeps windows clear of it; notches float over them.
    { path: ["surfaces", s, "reserve_space"], value: style === "always" },
  ];
}

function go(next: Step, host: Host): void {
  step = next;
  host.render();
  document.querySelector(".setup-page")?.scrollTo(0, 0);
}

function header(title: string, sub: string): HTMLElement {
  return $("div", "setup-head", $("h1", "setup-title", title), $("p", "setup-sub", sub));
}

function card(...kids: (HTMLElement | null)[]): HTMLElement {
  return $("section", "card", ...kids.filter((k): k is HTMLElement => k !== null));
}

function welcome(host: Host): HTMLElement {
  const logo = kasumiLogo();
  logo.classList.add("setup-logo");
  const button = (label: string, sub: string, cls: string, onClick: () => void) => {
    const b = $("button", `setup-choice ${cls}`, $("span", "setup-choice-label", label), $("span", "setup-choice-sub", sub));
    b.addEventListener("click", onClick);
    return b;
  };
  return $(
    "div",
    "setup-welcome",
    logo,
    $("h1", "setup-title", "Welcome to Kasumi"),
    $("p", "setup-sub", "A notch-style taskbar and Start menu for Windows."),
    $(
      "div",
      "setup-choices",
      button("First time quick setup", "A few steps: startup, your bars, widgets and pop-ups.", "primary", () => {
        flow = "quick";
        go("general", host);
      }),
      button("Continue with defaults", "Keep things as they are; everything can be changed in Settings.", "", () => getCurrentWindow().close()),
      button("Try out presets", "Pick a ready-made look and try it right away.", "", () => {
        flow = "presets";
        go("presets", host);
      }),
    ),
  );
}

function presets(host: Host): HTMLElement {
  const grid = $("div", "setup-presets");
  const fill = () => {
    grid.replaceChildren(
      ...(shipped ?? []).map((p) => {
        const b = $("button", `setup-preset${picked === p.name ? " active" : ""}`, screen(barShapes(p.top, "top") + barShapes(p.bottom, "bottom")), $("span", "setup-preset-name", p.name));
        b.addEventListener("click", async () => {
          try {
            await invoke("preset_apply", { name: p.name });
            picked = p.name;
            await host.reload();
          } catch (e) {
            host.toast(String(e), true);
          }
        });
        return b;
      }),
    );
  };
  if (shipped) fill();
  else
    invoke<{ name: string; top: Style; bottom: Style }[]>("presets_shipped")
      .then((list) => {
        shipped = list;
        fill();
      })
      .catch((e) => host.toast(String(e), true));
  return $("div", "", header("Try out presets", "Click one to switch to it; your screen changes right away. Pick the one you like, then Next."), grid);
}

function general(host: Host): HTMLElement {
  const launcherOn = (cfg: Cfg) => cfg.launcher?.enabled !== false;
  const fields: Field[] = [
    { kind: "autostart", label: "Start with Windows" },
    { kind: "toggle", label: "Rounded screen corners", help: "Rounds the corners of your monitors.", path: ["general", "rounded_corners"], default: false },
    { kind: "toggle", label: "Launcher", help: "Search and start apps from the bar.", path: ["launcher", "enabled"], default: true },
    {
      kind: "toggle",
      label: "Replace the Start menu",
      help: "The Windows key opens the launcher instead of Start. Win + other keys work as usual.",
      path: (cfg) => (launcherOn(cfg) ? ["launcher", "start_menu"] : null),
      default: false,
    },
    {
      kind: "text",
      label: "Launcher hotkey",
      help: "Opens the launcher, like Win+Alt+Space or Ctrl+Alt+K. Empty for none.",
      path: (cfg) => (launcherOn(cfg) && cfg.launcher?.start_menu !== true ? ["launcher", "hotkey"] : null),
      default: "Win+Alt+Space",
      placeholder: "None",
    },
    { kind: "toggle", label: "Update automatically", help: "Installs new versions by itself (Kasumi restarts).", path: ["general", "auto_update"], default: false },
  ];
  return $("div", "", header("General", "The basics. All of these are also in Settings."), card(...fields.map((f) => host.control(f))));
}

function bar(host: Host, s: "top" | "bottom"): HTMLElement {
  const cfg = host.cfg();
  const current = styleOf(cfg, s);
  const cards = $(
    "div",
    "setup-styles",
    ...STYLES.map(([style, label, help]) => {
      const b = $("button", `setup-style${current === style ? " active" : ""}`, screen(barShapes(style, s)), $("span", "setup-style-name", label), $("span", "setup-style-help", help));
      b.addEventListener("click", () => host.saveMany(styleOps(s, style)));
      return b;
    }),
  );
  const parts: HTMLElement[] = [header(s === "top" ? "Top bar" : "Bottom bar", `How the ${s} of your screens looks.`), cards];
  if (s === "top" && (current === "bar" || current === "dock")) {
    // Corner notches over a maximized window's caption buttons: moved aside, or not.
    const avoid = cfg.notch?.left?.avoid_windows !== false && cfg.notch?.right?.avoid_windows !== false;
    const sw = toggleSwitch(avoid, (on) =>
      host.saveMany([
        { path: ["notch", "left", "avoid_windows"], value: on },
        { path: ["notch", "right", "avoid_windows"], value: on },
      ]),
    );
    parts.push(
      card(
        $(
          "div",
          "setup-aside",
          asideScreen(avoid),
          $(
            "div",
            "setup-aside-text",
            $("div", "row-label", "Move corner notches out of the way"),
            $("div", "row-help", "When a window is maximized, a corner notch slides aside so its minimize, maximize and close buttons stay clickable."),
            sw.el,
          ),
        ),
      ),
    );
  }
  return $("div", "", ...parts);
}

function widgets(host: Host): HTMLElement {
  const cfg = host.cfg();
  const shown = (["top", "bottom"] as const).filter((s) => styleOf(cfg, s) !== "off");
  if (!shown.length) return $("div", "", header("Widgets", "Both bars are off: turn one on to place widgets."));
  if (!shown.includes(scope.surface as "top")) scope.surface = shown[0];
  scope.monitor = null;
  const chips = $(
    "div",
    "scope-bar",
    ...shown.map((s) => {
      const c = $("button", `scope-chip${scope.surface === s ? " active" : ""}`, glyph(s === "top" ? 0xe70e : 0xe70d), s === "top" ? "Top bar" : "Bottom bar");
      c.addEventListener("click", () => {
        scope.surface = s;
        host.render();
      });
      return c;
    }),
  );
  const title = $("h2", "card-title", "Placement", $("span", "card-legend", ...CHIP_LEGEND.map(([code, text]) => $("span", "", glyph(code), text))));
  return $(
    "div",
    "",
    header("Widgets", "Drag widgets between the sides of a bar, to the other bar, or to Disabled."),
    chips,
    card(title, host.control({ kind: "custom", editor: "placement" })),
  );
}

function osd(host: Host): HTMLElement {
  const on = host.cfg().osd?.enabled !== false;
  return $(
    "div",
    "",
    header("Pop-ups", "Volume, brightness and other changes show as a notch that grows into a small pill."),
    card(
      host.control({ kind: "toggle", label: "Show pop-ups", help: "Takes over the Windows volume flyout.", path: ["osd", "enabled"], default: true }),
      on ? host.control({ kind: "custom", editor: "osdSpot" }) : null,
    ),
  );
}

function done(): HTMLElement {
  const logo = kasumiLogo();
  logo.classList.add("setup-logo");
  return $(
    "div",
    "setup-welcome",
    logo,
    $("h1", "setup-title", "Your Kasumi is set up"),
    $("p", "setup-sub", "There's much more to tweak in Settings: every widget, the look, animations. You can run this setup again from Settings › About."),
  );
}

export function renderSetup(host: Host): HTMLElement {
  for (const w of SETUP_HIDDEN) placementHide.add(w);
  const pages: Record<Step, () => HTMLElement> = {
    welcome: () => welcome(host),
    presets: () => presets(host),
    general: () => general(host),
    top: () => bar(host, "top"),
    bottom: () => bar(host, "bottom"),
    widgets: () => widgets(host),
    osd: () => osd(host),
    done,
  };
  const page = $("main", "setup-page", pages[step]());
  if (step === "welcome") return $("div", "setup", page);

  const order: Step[] = flow === "quick" ? ["welcome", ...QUICK, "done"] : ["welcome", "presets", "done"];
  const i = order.indexOf(step);
  const back = $("button", "button", "Back") as HTMLButtonElement;
  back.addEventListener("click", () => go(order[Math.max(0, i - 1)], host));
  const next = $("button", "button primary", step === "done" ? "Finish" : "Next") as HTMLButtonElement;
  next.addEventListener("click", () => (step === "done" ? getCurrentWindow().close() : go(order[i + 1], host)));
  const dots = $("div", "setup-dots", ...order.slice(1).map((s) => $("span", `setup-dot${s === step ? " active" : ""}`)));
  const foot = $("footer", "setup-foot", back, dots, next);
  if (step === "done") {
    const settings = $("button", "button", "Open settings") as HTMLButtonElement;
    settings.addEventListener("click", () => invoke("open_settings").then(() => getCurrentWindow().close()));
    foot.replaceChildren(back, dots, $("div", "setup-foot-end", settings, next));
  }
  return $("div", "setup", page, foot);
}

// --- Update notes ------------------------------------------------------------------

let notesBody: HTMLElement | null = null;

export function renderNotes(host: Host): HTMLElement {
  const version = new URLSearchParams(location.search).get("v") ?? "";
  if (!notesBody) {
    notesBody = $("div", "notes-loading", "Loading the notes…");
    const body = notesBody;
    invoke<string>("release_notes", { version })
      .then((md) => body.replaceChildren(md.trim() ? renderMarkdown(md) : $("p", "row-help", "This version has no notes.")))
      .catch((e) => {
        const open = $("button", "button", "Open on GitHub");
        open.addEventListener("click", () => invoke("run_action", { action: "open", target: `https://github.com/f0xh3x/Kasumi/releases/tag/v${version}` }));
        body.replaceChildren($("p", "row-help", String(e)), open);
      });
  }
  const logo = kasumiLogo();
  logo.classList.add("notes-logo");
  const close = $("button", "button primary", "Close");
  close.addEventListener("click", () => getCurrentWindow().close());
  const toggle = host.control({
    kind: "toggle",
    label: "Show what's new after updates",
    path: ["general", "update_notes"],
    default: true,
  });
  return $(
    "div",
    "setup",
    $("main", "setup-page notes", $("div", "notes-head", logo, $("h1", "setup-title", `What's new in Kasumi ${version}`)), notesBody),
    $("footer", "setup-foot", toggle ?? $("span"), close),
  );
}
