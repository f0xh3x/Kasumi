// The first-time setup and the update notes: views of the settings page in their
// own windows (settings.html?view=setup / ?view=notes, opened by settings.rs).
// Everything they change is saved at once, like in the settings window.
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { kasumiLogo } from "../quick-tiles";
import { $, glyph, toggleSwitch } from "./dom";
import { CHIP_LEGEND, placementHide, scope, taskbarGuide, type Op } from "./editors";
import { renderMarkdown } from "./markdown";
import type { Cfg, Field } from "./schema";
import bottomAlways from "./previews/bottom-always.svg";
import bottomBar from "./previews/bottom-bar.svg";
import bottomDock from "./previews/bottom-dock.svg";
import offPic from "./previews/off.svg";
import presetStatusBar from "./previews/preset-classic-status-bar.svg";
import presetTaskbar from "./previews/preset-classic-taskbar.svg";
import presetDefault from "./previews/preset-default.svg";
import presetModern from "./previews/preset-modern.svg";
import topAlways from "./previews/top-always.svg";
import topBar from "./previews/top-bar.svg";
import topDock from "./previews/top-dock.svg";

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

/** Each bar style's picture, per edge, and each shipped preset's (previews/*.svg). */
const STYLE_PICS: Record<"top" | "bottom", Record<Style, string>> = {
  top: { bar: topBar, dock: topDock, always: topAlways, off: offPic },
  bottom: { bar: bottomBar, dock: bottomDock, always: bottomAlways, off: offPic },
};
const PRESET_PICS: Record<string, string> = {
  Default: presetDefault,
  Modern: presetModern,
  "Classic status bar": presetStatusBar,
  "Classic taskbar": presetTaskbar,
};

/** A screen picture (an image: the files share class names, which inline SVGs would mix up). */
function picture(src: string): HTMLElement {
  const img = $("img") as HTMLImageElement;
  img.src = src;
  img.alt = "";
  img.draggable = false;
  return $("div", "setup-screen", img);
}

/** A maximized window under the top bar, in the previews' style: its caption buttons
 *  in the top-right corner, the corner notch over them or moved aside. */
function asideScreen(avoid: boolean): HTMLElement {
  const box = $("div", "setup-screen");
  const ink = "#0e0e0e";
  // The previews' corner notches (previews/top-bar.svg).
  const left = `<rect fill="${ink}" width="13.39" height="7.38"/><path fill="${ink}" d="M11.94,7.38h1.7C16.56,7.38,17.9,0,22.35,0h-10.41v7.38Z"/>`;
  // In the corner it reaches the edge (one curve); moved aside it stands free, curved on
  // both sides like the previews' middle notch, clear of the buttons.
  const right = avoid
    ? `<g transform="translate(12.3 0)"><rect fill="${ink}" x="61.43" width="27.33" height="7.38"/>` +
      `<path fill="${ink}" d="M63.43,7.38h-1.7C58.81,7.38,57.47,0,53.01,0h10.41v7.38Z"/><path fill="${ink}" d="M87.31,7.38h1.7C91.92,7.38,93.26,0,97.72,0h-10.41v7.38Z"/></g>`
    : `<rect fill="${ink}" x="137.34" width="13.39" height="7.38"/><path fill="${ink}" d="M138.79,7.38h-1.7C134.17,7.38,132.84,0,128.38,0h10.41v7.38Z"/>`;
  const arrow = avoid ? `<path d="M118 13.5h-7m2.6-2.6l-2.6 2.6 2.6 2.6" stroke="${ink}" stroke-width="1.1" fill="none" stroke-linecap="round" stroke-linejoin="round"/>` : "";
  box.innerHTML =
    '<svg viewBox="0 0 150.73 92.93" aria-hidden="true">' +
    // The window: title bar with minimize, maximize and close; a few lines of content.
    '<rect width="150.73" height="92.93" fill="#f4f4f4"/><rect width="150.73" height="12" fill="#e3e3e3"/>' +
    '<g stroke="#3a3a3a" stroke-width="0.9" fill="none"><path d="M116 6h5"/><rect x="127" y="3.8" width="4.4" height="4.4"/><path d="M138 3.8l4.4 4.4M142.4 3.8l-4.4 4.4"/></g>' +
    '<rect x="8" y="22" width="56" height="5" rx="2.5" fill="#d4d4d4"/><rect x="8" y="32" width="92" height="3.6" rx="1.8" fill="#e0e0e0"/><rect x="8" y="40" width="76" height="3.6" rx="1.8" fill="#e0e0e0"/>' +
    `${left}${right}${arrow}</svg>`;
  return box;
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
        const b = $("button", `setup-preset${picked === p.name ? " active" : ""}`, picture(PRESET_PICS[p.name] ?? offPic), $("span", "setup-preset-name", p.name));
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
      const b = $("button", `setup-style${current === style ? " active" : ""}`, picture(STYLE_PICS[s][style]), $("span", "setup-style-name", label), $("span", "setup-style-help", help));
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
    taskbarGuide({} as never),
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
