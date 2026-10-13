// The settings window is generated from this declaration: every setting is
// listed once, with where it lives in config.toml, its control and default.
// Adding a feature's settings = adding entries here.

import { CHIP_LEGEND, ownsLayout, scope } from "./editors";

export type Path = (string | number)[];
export type Cfg = Record<string, any>;
/** A fixed path, or one computed from the config (null hides the field). */
export type PathSpec = Path | ((cfg: Cfg) => Path | null);

interface Base {
  label: string;
  help?: string;
  path: PathSpec;
  default: unknown;
}

/** Animation effects per widget (Settings > Animation > Per widget): the ones that apply to each. */
const PANEL_FX = "cascade";
const ANIM_WIDGETS: [string, string, number, string][] = [
  ["clock", "Clock", 0xe823, `roll,${PANEL_FX}`],
  ["weather", "Weather", 0xe706, `roll,${PANEL_FX}`],
  ["temps", "PC temps", 0xe9ca, `roll,${PANEL_FX}`],
  ["media", "Media", 0xe8d6, `${PANEL_FX}`],
  ["notifications", "Notifications", 0xea8f, `roll,nudge,${PANEL_FX}`],
  ["quick", "Quick settings", 0xe713, `roll,${PANEL_FX}`],
  ["tasks", "Tasks", 0xe737, `magnify,glide,${PANEL_FX}`],
  ["tray", "Tray", 0xe70d, `${PANEL_FX}`],
  ["menu", "Menu", 0xe700, `${PANEL_FX}`],
  ["launcher", "Launcher", 0xe721, `${PANEL_FX}`],
  ["clipboard", "Clipboard", 0xe77f, `${PANEL_FX}`],
  ["privacy", "Privacy", 0xe720, `${PANEL_FX}`],
  ["claude", "Claude Code", 0xe8bd, `${PANEL_FX}`],
  ["netspeed", "Network speed", 0xe74b, "roll"],
  ["disk", "Disk usage", 0xeda2, "roll"],
];

/**
 * Settings > Appearance > Per widget: each widget's font, and its accent on
 * details where it has some (the last column).
 */
const APPEARANCE_WIDGETS: [string, string, number, boolean][] = [
  ["clock", "Clock and calendar", 0xe823, true],
  ["tasks", "Tasks", 0xe737, true],
  ["weather", "Weather", 0xe706, true],
  ["temps", "PC temps", 0xe9ca, true],
  ["media", "Media", 0xe8d6, true],
  ["quick", "Quick settings", 0xe713, true],
  ["menu", "Menu", 0xe700, true],
  ["launcher", "Launcher", 0xe721, true],
  ["tray", "Tray", 0xe70d, true],
  ["buttons", "Buttons", 0xe7e8, true],
  ["notifications", "Notifications", 0xea8f, true],
  ["clipboard", "Clipboard", 0xe77f, true],
  ["disk", "Disk usage", 0xeda2, true],
  ["claude", "Claude Code", 0xe8bd, true],
  ["workspaces", "Workspaces", 0xe8a9, true],
  ["focus", "Focused app", 0xe7c4, false],
  ["netspeed", "Network speed", 0xe74b, false],
  ["language", "Keyboard language", 0xe765, false],
];

export type Field =
  | (Base & { kind: "toggle" })
  | (Base & { kind: "slider"; min: number; max: number; step: number; unit?: string })
  | (Base & { kind: "number"; min?: number; max?: number; step?: number; unit?: string })
  | (Base & { kind: "text"; placeholder?: string })
  | (Base & { kind: "select"; options: { value: string; label: string }[] })
  | (Base & { kind: "color" })
  | (Base & { kind: "list"; placeholder?: string })
  | { kind: "autostart"; label: string; help?: string }
  | { kind: "action"; label: string; help?: string; button: string; run: string; confirm?: string }
  /** A structured editor from editors.ts. */
  | { kind: "custom"; editor: string };

/**
 * A setting of the surface selected on the Bars page: hidden when the surface is
 * off, and (`only`) for styles it doesn't apply to.
 */
const surfacePath =
  (key: string, only?: "dock" | "notched"): PathSpec =>
  (cfg) => {
    const s = cfg.surfaces?.[scope.surface];
    if (!s || s.enabled === false) return null;
    if (only === "dock" && s.mode !== "dock") return null;
    if (only === "notched" && s.mode === "always") return null;
    return ["surfaces", scope.surface, key];
  };

export interface Section {
  title: string;
  help?: string;
  fields: Field[];
  /** Shown as a collapsible row (closed by default) with this icon. */
  collapsible?: boolean;
  /** The card consecutive collapsible sections share (default "Widget settings"). */
  group?: string;
  icon?: number;
  /** Widgets page: only in the Global view, or only when a monitor is selected. */
  scope?: "global" | "monitor";
  /** A collapsible section shown inside the card before it (not in its own group). */
  attach?: boolean;
  /** A small tag beside the title, e.g. what the settings apply to. */
  badge?: (cfg: Cfg) => string | null;
  /** Icons explained on the right of the title: [glyph, meaning]. */
  legend?: [number, string][];
}

export interface Page {
  id: string;
  label: string;
  /** Segoe Fluent Icons codepoint. */
  icon: number;
  sections: Section[];
}

export const pages: Page[] = [
  {
    id: "general",
    label: "General",
    icon: 0xe713,
    sections: [
      {
        title: "Startup",
        fields: [{ kind: "autostart", label: "Start with Windows" }],
      },
      {
        title: "Behaviour",
        fields: [
          {
            kind: "toggle",
            label: "Hide in fullscreen",
            help: "Hide the notch while a game or video is fullscreen.",
            path: ["general", "hide_in_fullscreen"],
            default: true,
          },
          { kind: "toggle", label: "Tooltips", help: "Show a hint when hovering widgets in the bars.", path: ["general", "tooltips"], default: true },
          {
            kind: "toggle",
            label: "Hide the Windows taskbar",
            help: "Hidden while Kasumi runs and restored when it quits. The Win key and Start menu keep working. Turn off Windhawk mods that show the taskbar (e.g. “Taskbar auto-hide when maximized”).",
            path: ["shell", "hide_taskbar"],
            default: false,
          },
        ],
      },
      {
        title: "Screen corners",
        help: "Rounds the corners of your monitors. Where a notch or an always-on bar fills a corner, the rounding sits just past it.",
        fields: [
          { kind: "toggle", label: "Rounded screen corners", path: ["general", "rounded_corners"], default: false },
          {
            kind: "slider",
            label: "Roundness",
            path: (cfg) => (cfg.general?.rounded_corners ? ["general", "corner_radius"] : null),
            default: 10,
            min: 2,
            max: 40,
            step: 1,
            unit: "px",
          },
          {
            kind: "toggle",
            label: "Attached to the bars",
            help: "On: the corners are part of the bars — they follow their animations and take their tint and opacity. Off: pure black, always right in the screen corners.",
            path: (cfg) => (cfg.general?.rounded_corners ? ["general", "corners_attached"] : null),
            default: true,
          },
        ],
      },
      {
        title: "Launcher",
        help: "Search and start apps. With the Launcher widget placed it opens from there; without it, from the middle of the bottom (or top) bar.",
        fields: [
          { kind: "toggle", label: "Launcher", path: ["launcher", "enabled"], default: true },
          {
            kind: "toggle",
            label: "Replace the Start menu",
            help: "The Windows key opens the launcher instead of Start. Win + other keys (Win+E, Win+D...) work as usual.",
            path: (cfg) => (cfg.launcher?.enabled === false ? null : ["launcher", "start_menu"]),
            default: false,
          },
          {
            kind: "select",
            label: "Over a fullscreen app",
            help: "What the Windows key (or the hotkey) does while a game or video is fullscreen.",
            path: (cfg) => (cfg.launcher?.enabled === false ? null : ["launcher", "fullscreen"]),
            default: "launcher",
            options: [
              { value: "launcher", label: "Opens the launcher" },
              { value: "notches", label: "Shows the notches for 2 seconds (press again for the launcher)" },
            ],
          },
          {
            kind: "text",
            label: "Hotkey",
            help: "Like Win+Alt+Space or Ctrl+Alt+K; empty for none (the Windows key can open it instead). If it doesn't work, another app has the combination (the log says so).",
            path: (cfg) => (cfg.launcher?.enabled === false ? null : ["launcher", "hotkey"]),
            default: "Win+Alt+Space",
            placeholder: "None",
          },
        ],
      },
    ],
  },
  {
    id: "widgets",
    label: "Bars",
    icon: 0xe71d,
    sections: [
      { title: "Layout for", fields: [{ kind: "custom", editor: "scopeSelector" }] },
      {
        title: "Bar",
        scope: "global",
        badge: () => "All monitors",
        fields: [{ kind: "custom", editor: "surfaceManager" }],
      },
      {
        title: "Bar settings",
        scope: "global",
        collapsible: true,
        attach: true,
        icon: 0xe713,
        fields: [
          { kind: "custom", editor: "surfaceStyle" },
          {
            kind: "select",
            label: "Size at rest",
            help: "Tiny is a thin handle that opens on hover.",
            path: surfacePath("rest", "dock"),
            default: "small",
            options: [
              { value: "tiny", label: "Tiny" },
              { value: "small", label: "Small" },
              { value: "medium", label: "Medium" },
              { value: "large", label: "Large" },
            ],
          },
          {
            kind: "select",
            label: "Size when open",
            path: surfacePath("open", "dock"),
            default: "small",
            options: [
              { value: "small", label: "Small" },
              { value: "medium", label: "Medium" },
              { value: "large", label: "Large" },
            ],
          },
          {
            kind: "toggle",
            label: "Shrink when a window covers it",
            help: "Shrinks to the tiny handle while a maximized window is under it.",
            path: surfacePath("shrink_when_covered", "dock"),
            default: true,
          },
          {
            kind: "toggle",
            label: "Reserve space",
            help: "Keep the bar's strip of the screen like the taskbar does: maximized windows stop at it instead of going under it.",
            path: surfacePath("reserve_space"),
            default: false,
          },
          { kind: "toggle", label: "Hot corner: left", help: "Resting the cursor in the corner opens the bar.", path: surfacePath("hot_left", "notched"), default: false },
          { kind: "toggle", label: "Hot corner: right", path: surfacePath("hot_right", "notched"), default: false },
          {
            kind: "toggle",
            label: "Anywhere along the edge",
            help: "Pushing the cursor against the bar's screen edge opens it, wherever it is.",
            path: surfacePath("hot_edge", "notched"),
            default: false,
          },
          { kind: "slider", label: "Hot corner size", path: surfacePath("hot_corner_size", "notched"), default: 12, min: 4, max: 40, step: 1, unit: "px" },
          { kind: "slider", label: "Edge zone size", help: "How close to the edge counts.", path: surfacePath("hot_edge_size", "notched"), default: 2, min: 1, max: 30, step: 1, unit: "px" },
        ],
      },
      { title: "This monitor", scope: "monitor", badge: () => `${scope.monitor} only`, fields: [{ kind: "custom", editor: "monitorToggles" }] },
      {
        title: "Placement",
        // Which layout this edits: the global one, or the selected monitor's own.
        badge: (cfg) =>
          !scope.monitor ? "Global layout" : ownsLayout(cfg, scope.monitor) ? `${scope.monitor}'s own layout` : "Global layout · a change here gives it its own",
        legend: CHIP_LEGEND,
        fields: [{ kind: "custom", editor: "placement" }],
      },
    ],
  },
  {
    id: "widget-settings",
    label: "Widgets",
    icon: 0xe74c,
    sections: [
      {
        title: "Clock",
        collapsible: true,
        icon: 0xe823,
        fields: [{ kind: "custom", editor: "clockFormat" }],
      },
      {
        title: "PC temps",
        collapsible: true,
        icon: 0xe9ca,
        fields: [
          {
            kind: "text",
            label: "LibreHardwareMonitor address",
            help: "LibreHardwareMonitor > Options > Remote Web Server.",
            path: ["widgets", "temps", "url"],
            default: "http://localhost:8085/data.json",
          },
          { kind: "slider", label: "Warning temperature", path: ["widgets", "temps", "warn"], default: 90, min: 50, max: 105, step: 1, unit: "°" },
          { kind: "slider", label: "Update every", path: ["widgets", "temps", "interval_ms"], default: 1000, min: 250, max: 5000, step: 250, unit: "ms" },
          { kind: "custom", editor: "tempsSensors" },
        ],
      },
      {
        title: "Weather",
        collapsible: true,
        icon: 0xe706,
        fields: [
          { kind: "custom", editor: "weatherLocation" },
          {
            kind: "select",
            label: "Units",
            path: ["widgets", "weather", "units"],
            default: "metric",
            options: [
              { value: "metric", label: "Celsius" },
              { value: "imperial", label: "Fahrenheit" },
            ],
          },
          {
            kind: "select",
            label: "Panel shows",
            path: ["widgets", "weather", "forecast"],
            default: "hourly",
            options: [
              { value: "hourly", label: "The next hours" },
              { value: "daily", label: "The next days (high and low)" },
            ],
          },
          {
            kind: "slider",
            label: "Hours in the panel",
            path: (cfg) => (cfg.widgets?.weather?.forecast === "daily" ? null : ["widgets", "weather", "hours"]),
            default: 8,
            min: 3,
            max: 12,
            step: 1,
          },
          { kind: "slider", label: "Refresh every", path: ["widgets", "weather", "interval_min"], default: 15, min: 5, max: 60, step: 5, unit: "min" },
        ],
      },
      {
        title: "Media",
        collapsible: true,
        icon: 0xe8d6,
        fields: [
          {
            kind: "select",
            label: "Playback keys",
            help: "Previous / play-pause / next beside the title. They're always in the media panel (click the title).",
            path: ["widgets", "media", "keys"],
            default: "off",
            options: [
              { value: "off", label: "Off" },
              { value: "always", label: "Always" },
              { value: "open", label: "Only while the bar is open" },
              { value: "closed", label: "Only while the bar is closed" },
            ],
          },
          {
            kind: "select",
            label: "Keys position",
            path: (cfg) => ((cfg.widgets?.media?.keys ?? "off") === "off" || cfg.widgets?.media?.keys === false ? null : ["widgets", "media", "keys_side"]),
            default: "left",
            options: [
              { value: "left", label: "Left of the title" },
              { value: "right", label: "Right of the title" },
            ],
          },
          {
            kind: "toggle",
            label: "Fixed title width",
            help: "The title always takes the full title width, so its notch keeps one size whatever plays.",
            path: ["widgets", "media", "fixed_width"],
            default: false,
          },
          {
            kind: "select",
            label: "Title alignment",
            help: "Where a title shorter than the title width sits.",
            path: (cfg) => (cfg.widgets?.media?.fixed_width ? ["widgets", "media", "title_align"] : null),
            default: "center",
            options: [
              { value: "left", label: "Left" },
              { value: "center", label: "Center" },
              { value: "right", label: "Right" },
            ],
          },
          { kind: "slider", label: "Title width", path: ["widgets", "media", "max_width"], default: 240, min: 100, max: 500, step: 10, unit: "px" },
          {
            kind: "toggle",
            label: "Scroll long titles",
            help: "Titles wider than the title width scroll instead of ending in “…”.",
            path: ["widgets", "media", "scroll"],
            default: false,
          },
          {
            kind: "slider",
            label: "Scroll speed",
            path: (cfg) => (cfg.widgets?.media?.scroll === true ? ["widgets", "media", "scroll_speed"] : null),
            default: 32,
            min: 10,
            max: 80,
            step: 2,
            unit: "px/s",
          },
        ],
      },
      {
        title: "Launcher",
        collapsible: true,
        icon: 0xe721,
        help: "A search icon that becomes the launcher's search field. These settings are for the launcher however it opens; its hotkey and the Start menu option are under General.",
        fields: [
          { kind: "slider", label: "Results shown", help: "How many apps the search lists at most.", path: ["widgets", "launcher", "results"], default: 8, min: 3, max: 20, step: 1 },
          { kind: "slider", label: "Width", path: ["widgets", "launcher", "panel_width"], default: 340, min: 260, max: 600, step: 10, unit: "px" },
          { kind: "custom", editor: "launcherPower" },
          {
            kind: "select",
            label: "Power buttons position",
            path: (cfg) => ((cfg.widgets?.launcher?.power ?? []).length ? ["widgets", "launcher", "power_align"] : null),
            default: "right",
            options: [
              { value: "left", label: "Left" },
              { value: "center", label: "Middle" },
              { value: "right", label: "Right" },
            ],
          },
          { kind: "toggle", label: "User button", help: "Your account picture and name beside the power buttons; opens Settings › Your info.", path: ["widgets", "launcher", "user"], default: false },
        ],
      },
      {
        title: "Focused app",
        collapsible: true,
        icon: 0xe7c4,
        help: "The window in front: its title, its app's name, or both.",
        fields: [
          {
            kind: "select",
            label: "Shows",
            path: ["widgets", "focus", "show"],
            default: "title",
            options: [
              { value: "title", label: "Window title" },
              { value: "app", label: "App name" },
              { value: "both", label: "App name — window title" },
            ],
          },
          { kind: "slider", label: "Width", help: "Longer titles end in “…”.", path: ["widgets", "focus", "max_width"], default: 300, min: 100, max: 800, step: 10, unit: "px" },
        ],
      },
      {
        title: "Workspaces (Komorebi)",
        collapsible: true,
        icon: 0xe8a9,
        help: "The Komorebi tiling window manager's workspaces on each bar's monitor. Click one to switch to it, or scroll over them. Hidden while Komorebi isn't running.",
        fields: [
          {
            kind: "select",
            label: "Shows",
            path: ["widgets", "workspaces", "show"],
            default: "dots",
            options: [
              { value: "dots", label: "Dots (the current one a pill)" },
              { value: "numbers", label: "Numbers" },
              { value: "names", label: "Names" },
            ],
          },
          { kind: "toggle", label: "Hide empty workspaces", help: "The current one always shows.", path: ["widgets", "workspaces", "hide_empty"], default: false },
        ],
      },
      {
        title: "Network speed",
        collapsible: true,
        icon: 0xe74b,
        help: "Live download and upload speed of your network adapters.",
        fields: [
          {
            kind: "select",
            label: "Shows",
            path: ["widgets", "netspeed", "show"],
            default: "both",
            options: [
              { value: "both", label: "Download and upload" },
              { value: "down", label: "Download" },
              { value: "up", label: "Upload" },
            ],
          },
          {
            kind: "select",
            label: "Units",
            path: ["widgets", "netspeed", "units"],
            default: "bytes",
            options: [
              { value: "bytes", label: "Bytes (MB/s)" },
              { value: "bits", label: "Bits (Mb/s)" },
            ],
          },
        ],
      },
      {
        title: "Disk usage",
        collapsible: true,
        icon: 0xeda2,
        help: "How full your drives are. A click opens This PC.",
        fields: [
          { kind: "custom", editor: "diskPicker" },
          {
            kind: "select",
            label: "Indicator",
            path: ["widgets", "disk", "indicator"],
            default: "bar",
            options: [
              { value: "bar", label: "Bar" },
              { value: "circle", label: "Ring" },
              { value: "none", label: "None" },
            ],
          },
          { kind: "slider", label: "Bar length", path: (cfg) => ((cfg.widgets?.disk?.indicator ?? "bar") === "bar" ? ["widgets", "disk", "bar_length"] : null), default: 30, min: 16, max: 80, step: 2, unit: "px" },
          {
            kind: "select",
            label: "Text",
            path: ["widgets", "disk", "show"],
            default: "percent",
            options: [
              { value: "percent", label: "Percent full" },
              { value: "used", label: "Used / total (GB)" },
              { value: "free", label: "Free (GB)" },
              { value: "none", label: "None" },
            ],
          },
          { kind: "toggle", label: "Drive letters", path: ["widgets", "disk", "letters"], default: true },
        ],
      },
      {
        title: "Privacy",
        collapsible: true,
        icon: 0xe720,
        help: "Shows only while an app uses your microphone, camera or location; click for which apps.",
        fields: [
          { kind: "toggle", label: "Microphone", path: ["widgets", "privacy", "mic"], default: true },
          { kind: "toggle", label: "Camera", path: ["widgets", "privacy", "camera"], default: true },
          { kind: "toggle", label: "Location", path: ["widgets", "privacy", "location"], default: true },
        ],
      },
      {
        title: "Notifications",
        collapsible: true,
        icon: 0xea8f,
        help: "Windows' notifications: the bell opens a list of recent ones. Windows keeps none while notifications are switched off in its settings (Do not disturb is fine: it only stops Windows' own pop-ups). New ones can also pop out of the notch: OSD page.",
        fields: [
          {
            kind: "select",
            label: "Clicking a notification",
            path: ["widgets", "notifications", "click"],
            default: "open",
            options: [
              { value: "open", label: "Opens the app that sent it" },
              { value: "nothing", label: "Does nothing" },
            ],
          },
          {
            kind: "toggle",
            label: "Keep until cleared",
            help: "Off: the list mirrors Windows, so what an app takes back (Discord once you've read the message) goes. On: Kasumi keeps them until you dismiss them (× or Clear all).",
            path: ["widgets", "notifications", "keep"],
            default: false,
          },
          {
            kind: "toggle",
            label: "Clear after checking",
            help: "Opening the panel and closing it clears what it showed (from Kasumi's list, not Windows').",
            path: ["widgets", "notifications", "clear_on_close"],
            default: false,
          },
          { kind: "toggle", label: "Count", help: "How many notifications there are, beside the bell.", path: ["widgets", "notifications", "count"], default: true },
          { kind: "color", label: "New notifications", help: "The bell's color while there's something you haven't looked at.", path: ["widgets", "notifications", "color"], default: "#4cc2ff" },
        ],
      },
      {
        title: "Clipboard",
        collapsible: true,
        icon: 0xe77f,
        help: "Recent copies, to paste again: click one and it's pasted into the app you're in. Kept in memory only (never saved), and copies apps mark as private (passwords) are left out.",
        fields: [
          { kind: "slider", label: "Remembers", path: ["widgets", "clipboard", "size"], default: 20, min: 5, max: 100, step: 5, unit: " items" },
          { kind: "toggle", label: "Images", help: "Also keep copied images (up to 8).", path: ["widgets", "clipboard", "images"], default: true },
          { kind: "toggle", label: "Item count", path: ["widgets", "clipboard", "count"], default: false },
        ],
      },
      {
        title: "Claude Code",
        collapsible: true,
        icon: 0xe8bd,
        help: "Your Claude Code usage: your plan's limits from your Claude account, and the token details from Claude Code's session files on this PC.",
        fields: [
          {
            kind: "toggle",
            label: "Usage from your Claude account",
            help: "The same percentages as /usage, using Claude Code's sign-in on this PC (only read, never changed). Off, or signed out: the limits you set below.",
            path: ["widgets", "claude", "account"],
            default: true,
          },
          { kind: "toggle", label: "Text", path: ["widgets", "claude", "text"], default: true },
          { kind: "toggle", label: "5-hour window usage", path: (cfg) => (cfg.widgets?.claude?.text === false ? null : ["widgets", "claude", "show_window"]), default: true },
          { kind: "toggle", label: "Time until it resets", path: (cfg) => (cfg.widgets?.claude?.text === false ? null : ["widgets", "claude", "show_reset"]), default: true },
          { kind: "toggle", label: "Today's usage", path: (cfg) => (cfg.widgets?.claude?.text === false ? null : ["widgets", "claude", "show_today"]), default: false },
          { kind: "toggle", label: "Weekly usage left", help: "From your Claude account; without it, needs the weekly limit (else: the week's usage).", path: (cfg) => (cfg.widgets?.claude?.text === false ? null : ["widgets", "claude", "show_week_left"]), default: false },
          {
            kind: "select",
            label: "Indicator",
            path: ["widgets", "claude", "indicator"],
            default: "bar",
            options: [
              { value: "bar", label: "Bar" },
              { value: "circle", label: "Circle" },
              { value: "none", label: "None" },
            ],
          },
          {
            kind: "select",
            label: "Indicator shows",
            help: "Without a 5-hour limit set, it shows how far through the window you are.",
            path: (cfg) => (cfg.widgets?.claude?.indicator === "none" ? null : ["widgets", "claude", "indicator_for"]),
            default: "window",
            options: [
              { value: "window", label: "5-hour window" },
              { value: "week", label: "Last 7 days" },
            ],
          },
          { kind: "slider", label: "Bar length", path: (cfg) => ((cfg.widgets?.claude?.indicator ?? "bar") === "bar" ? ["widgets", "claude", "bar_length"] : null), default: 40, min: 16, max: 160, step: 2, unit: "px" },
          {
            kind: "select",
            label: "Indicator position",
            path: (cfg) => (cfg.widgets?.claude?.indicator === "none" || cfg.widgets?.claude?.text === false ? null : ["widgets", "claude", "indicator_side"]),
            default: "left",
            options: [
              { value: "left", label: "Left of the text" },
              { value: "right", label: "Right of the text" },
            ],
          },
          { kind: "toggle", label: "Working dot", help: "Pulses while a Claude Code session is busy.", path: ["widgets", "claude", "working"], default: true },
          { kind: "toggle", label: "Last 7 days in the panel", path: ["widgets", "claude", "weekly"], default: true },
          { kind: "number", label: "5-hour limit", help: "Tokens per 5-hour window (0: not set).", path: (cfg) => (cfg.widgets?.claude?.account === false ? ["widgets", "claude", "window_limit"] : null), default: 0, min: 0, max: 1000000000, step: 100000 },
          { kind: "number", label: "Weekly limit", help: "Tokens per 7 days (0: not set).", path: (cfg) => (cfg.widgets?.claude?.account === false ? ["widgets", "claude", "weekly_limit"] : null), default: 0, min: 0, max: 10000000000, step: 1000000 },
        ],
      },
      {
        title: "Quick settings",
        collapsible: true,
        icon: 0xe713,
        help: "Its panel has volume and brightness, battery and network, and shortcuts to Windows settings.",
        fields: [
          { kind: "custom", editor: "quickIcon" },
          { kind: "toggle", label: "Volume slider", help: "Off if you set the volume elsewhere (an audio interface's knob...).", path: ["widgets", "quick", "volume"], default: true },
          {
            kind: "toggle",
            label: "Output device",
            help: "A button before the volume slider: switch between speakers, headphones...",
            path: (cfg) => (cfg.widgets?.quick?.volume === false ? null : ["widgets", "quick", "outputs"]),
            default: false,
          },
          {
            kind: "toggle",
            label: "Per-app volume",
            help: "A chevron before the volume slider: a volume slider for each app playing sound.",
            path: (cfg) => (cfg.widgets?.quick?.volume === false ? null : ["widgets", "quick", "apps"]),
            default: false,
          },
          { kind: "toggle", label: "Brightness slider", help: "Shown only on screens that report brightness (laptops).", path: ["widgets", "quick", "brightness"], default: true },
          {
            kind: "toggle",
            label: "Display button",
            help: "A button before the brightness slider that opens display settings.",
            path: (cfg) => (cfg.widgets?.quick?.brightness === false ? null : ["widgets", "quick", "display"]),
            default: false,
          },
          { kind: "custom", editor: "quickTiles" },
        ],
      },
      {
        title: "Buttons",
        collapsible: true,
        icon: 0xe7e8,
        fields: [{ kind: "custom", editor: "buttonsWidgets" }],
      },
      {
        title: "Tasks",
        collapsible: true,
        icon: 0xe737,
        help: "Open apps. Click: switch to it (again: minimize). Middle click: close. Right click: its windows, new window, close all.",
        fields: [
          {
            kind: "select",
            label: "Windows from",
            path: ["widgets", "tasks", "monitor"],
            default: "own",
            options: [
              { value: "own", label: "This monitor" },
              { value: "all", label: "All monitors" },
            ],
          },
          {
            kind: "select",
            label: "Icons",
            help: "Monochrome keeps the detail in grey; Accent paints each icon's shape in the accent colour (Appearance).",
            path: ["widgets", "tasks", "icons"],
            default: "color",
            options: [
              { value: "color", label: "Color" },
              { value: "mono", label: "Monochrome" },
              { value: "accent", label: "Accent color" },
            ],
          },
          {
            kind: "toggle",
            label: "Dim minimized apps",
            help: "Apps whose windows are all minimized show a faded icon.",
            path: ["widgets", "tasks", "dim_minimized"],
            default: false,
          },
          {
            kind: "toggle",
            label: "Pinned apps",
            help: "Pinned apps come first, even when they aren't running; right-click an app to pin or unpin it. Starts with your Windows taskbar pins.",
            path: ["widgets", "tasks", "pinning"],
            default: true,
          },
          {
            kind: "select",
            label: "Labels",
            path: ["widgets", "tasks", "labels"],
            default: "none",
            options: [
              { value: "none", label: "Icons only" },
              { value: "active", label: "Title of the window in front" },
            ],
          },
        ],
      },
      {
        title: "Tray",
        collapsible: true,
        icon: 0xe70d,
        fields: [
          {
            kind: "toggle",
            label: "Host tray icons",
            help: "Collect apps' tray icons (Explorer's tray keeps working). Applies on the next launch.",
            path: ["widgets", "tray", "host"],
            default: true,
          },
          {
            kind: "select",
            label: "Show",
            path: ["widgets", "tray", "mode"],
            default: "collapsed",
            options: [
              { value: "collapsed", label: "Collapsed: a button with the icons in a panel" },
              { value: "expanded", label: "Expanded: every icon in the bar" },
            ],
          },
          {
            kind: "select",
            label: "Icons",
            help: "Monochrome keeps the detail in grey; Accent paints each icon's shape in the accent colour (Appearance).",
            path: ["widgets", "tray", "icons"],
            default: "color",
            options: [
              { value: "color", label: "Color" },
              { value: "mono", label: "Monochrome" },
              { value: "accent", label: "Accent color" },
            ],
          },
          {
            kind: "slider",
            label: "Icons per row",
            help: "In the panel.",
            path: (cfg) => (cfg.widgets?.tray?.mode === "expanded" ? null : ["widgets", "tray", "icons_per_row"]),
            default: 5,
            min: 3,
            max: 10,
            step: 1,
          },
          {
            kind: "list",
            label: "Hidden apps",
            help: "Process names, comma separated.",
            path: ["widgets", "tray", "hide"],
            default: ["explorer.exe"],
            placeholder: "explorer.exe",
          },
        ],
      },
      {
        title: "Keyboard language",
        collapsible: true,
        icon: 0xe765,
        fields: [
          { kind: "toggle", label: "Keyboard icon", help: "A keyboard before the language code.", path: ["widgets", "language", "icon"], default: false },
          {
            kind: "select",
            label: "Code",
            help: "Also used by the popup when you switch layouts.",
            path: ["widgets", "language", "format"],
            default: "three",
            options: [
              { value: "three", label: "ENG (three letters)" },
              { value: "two", label: "EN (two letters)" },
            ],
          },
        ],
      },
      {
        title: "Menu",
        collapsible: true,
        icon: 0xe700,
        fields: [{ kind: "custom", editor: "menuItems" }],
      },
    ],
  },
  {
    id: "notches",
    label: "Notches",
    icon: 0xe8fd,
    sections: [
      {
        title: "All notches",
        help: "Widgets pinned in Bars › Placement show as a notch on their side while the bar is closed.",
        fields: [
          {
            kind: "toggle",
            label: "Merge neighbouring notches",
            help: "Notch widgets on the same side share one notch; when the bar opens they spread out to their places.",
            path: ["notch", "merge"],
            default: true,
          },
          {
            kind: "toggle",
            label: "Combine overlapping notches",
            help: "Notches that would touch or overlap (e.g. on a narrow, vertical monitor) are drawn as one; widgets keep their places.",
            path: ["notch", "combine_overlapping"],
            default: true,
          },
        ],
      },
      {
        title: "Left notch",
        fields: [
          { kind: "toggle", label: "Sit in the corner", path: ["notch", "left", "hug"], default: true },
          {
            kind: "toggle",
            label: "Move aside for windows",
            help: "Slide to the offset while a maximized window's caption buttons are there (back in the corner while the bar is open).",
            path: ["notch", "left", "avoid_windows"],
            default: true,
          },
          { kind: "number", label: "Offset", help: "From the left edge, when moved aside or not in the corner.", path: ["notch", "left", "offset"], default: 150, min: 0, max: 2000, step: 1, unit: "px" },
        ],
      },
      {
        title: "Center notch",
        fields: [
          { kind: "number", label: "Offset", help: "Shift from the middle.", path: ["notch", "center", "offset"], default: 0, min: -2000, max: 2000, step: 1, unit: "px" },
        ],
      },
      {
        title: "Right notch",
        fields: [
          { kind: "toggle", label: "Sit in the corner", path: ["notch", "right", "hug"], default: true },
          {
            kind: "toggle",
            label: "Move aside for windows",
            help: "Slide to the offset while a maximized window's caption buttons are there (back in the corner while the bar is open).",
            path: ["notch", "right", "avoid_windows"],
            default: true,
          },
          { kind: "number", label: "Offset", help: "From the right edge, when moved aside or not in the corner.", path: ["notch", "right", "offset"], default: 150, min: 0, max: 2000, step: 1, unit: "px" },
        ],
      },
    ],
  },
  {
    id: "appearance",
    label: "Appearance",
    icon: 0xe771,
    sections: [
      {
        title: "Colors",
        fields: [
          {
            kind: "color",
            label: "Background tint",
            help: "The colour of the bars, notches and panels; with less than full opacity it tints what's behind them.",
            path: ["theme", "background"],
            default: "#000000",
          },
          {
            kind: "slider",
            label: "Opacity",
            help: "Of the bars, notches and panels; text and icons stay solid.",
            path: ["theme", "opacity"],
            default: 100,
            min: 0,
            max: 100,
            step: 5,
            unit: "%",
          },
          { kind: "color", label: "Text", path: ["theme", "text"], default: "#ffffff" },
          { kind: "slider", label: "Secondary text opacity", path: ["theme", "dim_opacity"], default: 0.45, min: 0.2, max: 0.9, step: 0.05 },
          {
            kind: "toggle",
            label: "Use Windows' accent colour",
            help: "Follows Settings > Personalization > Colors, also when it changes.",
            path: ["theme", "accent_sync"],
            default: false,
          },
          {
            kind: "color",
            label: "Accent",
            help: "Today in the calendar, and widget details when that's on.",
            path: (cfg) => (cfg.theme?.accent_sync === true ? null : ["theme", "accent"]),
            default: "#ffffff",
          },
          {
            kind: "toggle",
            label: "Accent on widget details",
            help: "Hover highlights, the line under the app in front, sliders, bars and key readings in the accent colour. Each widget can differ under Per widget below.",
            path: ["theme", "accent_details"],
            default: false,
          },
          { kind: "color", label: "Warning", help: "Hot temperatures.", path: ["theme", "warn"], default: "#ff9f43" },
          { kind: "custom", editor: "fontPicker" },
          { kind: "custom", editor: "fontPicker::secondary" },
          { kind: "slider", label: "Text size", path: ["theme", "font_size"], default: 13, min: 11, max: 16, step: 0.5, unit: "px" },
        ],
      },
      {
        title: "Notch shape",
        fields: [
          { kind: "slider", label: "Notch height", path: ["geometry", "notch_height"], default: 30, min: 20, max: 48, step: 1, unit: "px" },
          { kind: "slider", label: "Flare width", help: "How far the notch flares out along the top edge.", path: ["geometry", "concave_radius"], default: 33, min: 0, max: 80, step: 0.5, unit: "px" },
          { kind: "slider", label: "Round-off width", help: "How gently the sides curve into the tip.", path: ["geometry", "notch_corner_radius"], default: 25.5, min: 0, max: 60, step: 0.5, unit: "px" },
        ],
      },
      {
        title: "Bar and panels",
        fields: [
          { kind: "slider", label: "Bar height", path: ["geometry", "expanded_height"], default: 30, min: 20, max: 48, step: 1, unit: "px" },
          { kind: "slider", label: "Panel fillet", help: "Where panels and dropdowns leave the bar.", path: ["geometry", "panel_concave_radius"], default: 10, min: 0, max: 30, step: 1, unit: "px" },
          { kind: "slider", label: "Panel corners", path: ["geometry", "panel_corner_radius"], default: 12, min: 0, max: 30, step: 1, unit: "px" },
          { kind: "slider", label: "Side padding", path: ["bar", "padding"], default: 18, min: 0, max: 60, step: 1, unit: "px" },
          { kind: "slider", label: "Gap between widgets", path: ["bar", "gap"], default: 18, min: 4, max: 40, step: 1, unit: "px" },
        ],
      },
      {
        title: "Bar sizes",
        help: "Heights of the named sizes bars and docks rest and open at.",
        fields: [
          { kind: "slider", label: "Tiny", help: "A bare handle (no content).", path: ["sizes", "tiny"], default: 12, min: 4, max: 24, step: 1, unit: "px" },
          { kind: "slider", label: "Small", path: ["sizes", "small"], default: 30, min: 20, max: 48, step: 1, unit: "px" },
          { kind: "slider", label: "Medium", path: ["sizes", "medium"], default: 45, min: 30, max: 72, step: 1, unit: "px" },
          { kind: "slider", label: "Large", path: ["sizes", "large"], default: 60, min: 40, max: 96, step: 1, unit: "px" },
        ],
      },
      ...APPEARANCE_WIDGETS.map(([w, title, icon, accent]) => ({
        title,
        collapsible: true,
        group: "Per widget",
        icon,
        fields: [
          { kind: "custom" as const, editor: `fontPicker:${w}` },
          { kind: "custom" as const, editor: `fontPicker:${w}:secondary` },
          ...(accent ? [{ kind: "custom" as const, editor: `accentWidget:${w}` }] : []),
        ],
      })),
    ],
  },
  {
    id: "animation",
    label: "Animation",
    icon: 0xe945,
    sections: [
      {
        title: "Style",
        fields: [
          {
            kind: "select",
            label: "Bar animation",
            path: ["animation", "style"],
            default: "drop",
            options: [
              { value: "drop", label: "Drop from the top edge" },
              { value: "spread", label: "Spread out of the notches" },
            ],
          },
          {
            kind: "select",
            label: "Bar content",
            help: "How the open bar's widgets show while it opens.",
            path: ["animation", "bar_content"],
            default: "slide",
            options: [
              { value: "slide", label: "Slide in with the bar" },
              { value: "reveal", label: "Stay in place, the bar uncovers them" },
              { value: "fade", label: "Stay in place and fade in" },
            ],
          },
          {
            kind: "select",
            label: "Panel content",
            help: "How a widget's panel (calendar, weather...) shows its content while it opens.",
            path: ["animation", "panel_content"],
            default: "reveal",
            options: [
              { value: "reveal", label: "Stays in place, the panel uncovers it" },
              { value: "fade", label: "Stays in place and fades in" },
            ],
          },
        ],
      },
      {
        title: "Speed",
        fields: [
          { kind: "slider", label: "Opening speed", help: "Faster or slower than the springs below, same bounce.", path: ["animation", "open_speed"], default: 1, min: 0.5, max: 2, step: 0.1, unit: "×" },
          { kind: "slider", label: "Closing speed", path: ["animation", "close_speed"], default: 1, min: 0.5, max: 2, step: 0.1, unit: "×" },
        ],
      },
      {
        title: "Effects",
        help: "For every widget's panel and numbers; each widget can differ under Per widget below.",
        fields: [
          { kind: "toggle", label: "Hover nudge", help: "A notch grows a little as the cursor comes near it.", path: ["animation", "hover_nudge"], default: false },
          { kind: "toggle", label: "Cascade", help: "A widget's panel shows its content one piece after another: calendar days, weather hours, notifications...", path: ["animation", "cascade"], default: false },
          { kind: "toggle", label: "Rolling numbers", help: "Clock, temperatures, battery, counts... roll to their new value.", path: ["animation", "rolling"], default: false },
          { kind: "toggle", label: "Dock magnify", help: "Task icons grow as the cursor passes over them.", path: ["animation", "magnify"], default: false },
          { kind: "toggle", label: "Focus marker glides", help: "The line under the app in front travels to the next app you switch to, narrowing to a dot halfway.", path: ["animation", "marker_glide"], default: false },
          { kind: "toggle", label: "Notification nudge", help: "The bell shakes for a new notification; the pop-up's icon bounces.", path: ["animation", "notification_nudge"], default: false },
        ],
      },
      {
        title: "Springs",
        help: "Higher stiffness is faster. With Bounce on, lower damping is bouncier.",
        fields: [
          {
            kind: "toggle",
            label: "Bounce",
            help: "Off: nothing overshoots, every move eases into place. On: a little bounce, as much as the damping allows.",
            path: ["animation", "bounce"],
            default: false,
          },
          { kind: "slider", label: "Bar stiffness", path: ["animation", "expand_stiffness"], default: 3440, min: 200, max: 6000, step: 20 },
          { kind: "slider", label: "Bar damping", path: ["animation", "expand_damping"], default: 92, min: 10, max: 160, step: 1 },
          { kind: "slider", label: "Spread / slide stiffness", path: ["animation", "spread_stiffness"], default: 2120, min: 200, max: 6000, step: 20 },
          { kind: "slider", label: "Spread / slide damping", path: ["animation", "spread_damping"], default: 80, min: 10, max: 160, step: 1 },
          { kind: "slider", label: "Panel stiffness", path: ["animation", "panel_stiffness"], default: 775, min: 200, max: 6000, step: 20 },
          { kind: "slider", label: "Panel damping", path: ["animation", "panel_damping"], default: 43, min: 10, max: 160, step: 1 },
        ],
      },
      ...ANIM_WIDGETS.map(([w, title, icon, effects]) => ({
        title,
        collapsible: true,
        group: "Per widget",
        icon,
        fields: [{ kind: "custom" as const, editor: `animWidget:${w}:${effects}` }],
      })),
    ],
  },
  {
    id: "behaviour",
    label: "Hover",
    icon: 0xe962,
    sections: [
      {
        title: "Opening and closing",
        fields: [
          { kind: "slider", label: "Open delay", help: "How long the cursor rests on a notch before the bar opens.", path: ["timing", "dwell_ms"], default: 10, min: 0, max: 500, step: 5, unit: "ms" },
          { kind: "slider", label: "Close delay", help: "How long the cursor may leave the bar before it closes.", path: ["timing", "grace_ms"], default: 300, min: 0, max: 1500, step: 25, unit: "ms" },
          { kind: "slider", label: "Top edge band", help: "Cursor distance from the top edge that counts as hovering a notch.", path: ["timing", "edge_px"], default: 8, min: 1, max: 20, step: 1, unit: "px" },
        ],
      },
    ],
  },
  {
    id: "osd",
    label: "OSD",
    icon: 0xe767,
    sections: [
      {
        title: "On-screen display",
        help: "Volume, brightness and the others grow out of a notch for a moment. Where they appear is set here for all of them; each can choose its own below.",
        fields: [
          { kind: "toggle", label: "Show pop-ups", path: ["osd", "enabled"], default: true },
          { kind: "custom", editor: "osdPlace" },
          { kind: "slider", label: "Duration", help: "Volume, brightness, Caps Lock and the keyboard layout.", path: ["osd", "duration_ms"], default: 1500, min: 500, max: 4000, step: 100, unit: "ms" },
          { kind: "slider", label: "Pill width", path: ["osd", "width"], default: 230, min: 160, max: 400, step: 5, unit: "px" },
          { kind: "slider", label: "Pill height", path: ["osd", "height"], default: 34, min: 24, max: 48, step: 1, unit: "px" },
        ],
      },
      {
        title: "Volume",
        collapsible: true,
        group: "Pop-ups",
        icon: 0xe767,
        fields: [
          {
            kind: "toggle",
            label: "Take over the volume keys",
            help: "Hides the Windows volume flyout; Kasumi sets the volume.",
            path: ["osd", "replace_volume_keys"],
            default: true,
          },
          { kind: "slider", label: "Volume step", path: ["osd", "volume_step"], default: 2, min: 1, max: 10, step: 1, unit: "%" },
          { kind: "custom", editor: "osdPlace:volume" },
        ],
      },
      {
        title: "Brightness",
        collapsible: true,
        group: "Pop-ups",
        icon: 0xe706,
        fields: [
          { kind: "toggle", label: "Show brightness", help: "The built-in screen's brightness (laptop Fn keys).", path: ["osd", "brightness"], default: true },
          { kind: "custom", editor: "osdPlace:brightness" },
        ],
      },
      {
        title: "Caps Lock",
        collapsible: true,
        group: "Pop-ups",
        icon: 0xe765,
        fields: [
          { kind: "toggle", label: "Show Caps Lock", help: "When it's switched on or off.", path: ["osd", "caps_lock"], default: false },
          { kind: "custom", editor: "osdPlace:caps" },
        ],
      },
      {
        title: "Keyboard layout",
        collapsible: true,
        group: "Pop-ups",
        icon: 0xe765,
        fields: [
          { kind: "toggle", label: "Show layout changes", path: ["osd", "keyboard"], default: true },
          { kind: "slider", label: "Width at the widget", path: ["osd", "keyboard_width"], default: 120, min: 80, max: 200, step: 5, unit: "px" },
          { kind: "custom", editor: "osdPlace:keyboard" },
        ],
      },
      {
        title: "Notifications",
        collapsible: true,
        group: "Pop-ups",
        icon: 0xea8f,
        help: "New Windows notifications (not over fullscreen apps).",
        fields: [
          { kind: "custom", editor: "windowsGuide" },
          { kind: "toggle", label: "Show new notifications", path: ["osd", "notifications"], default: false },
          {
            kind: "select",
            label: "Look",
            path: (cfg) => (cfg.osd?.notifications === true ? ["osd", "notification_style"] : null),
            default: "line",
            options: [
              { value: "line", label: "One line: app, title and text" },
              { value: "two", label: "Two lines: title over the text" },
              { value: "compact", label: "Just the app's icon and name" },
            ],
          },
          {
            kind: "slider",
            label: "Shown for",
            path: (cfg) => (cfg.osd?.notifications === true ? ["osd", "notification_ms"] : null),
            default: 5000,
            min: 2000,
            max: 15000,
            step: 500,
            unit: "ms",
          },
          {
            kind: "toggle",
            label: "Fit to the text",
            help: "As wide as the notification needs, up to the width below. Off: always that wide.",
            path: (cfg) => (cfg.osd?.notifications === true ? ["osd", "notification_fit"] : null),
            default: true,
          },
          {
            kind: "slider",
            label: "Width",
            path: (cfg) => (cfg.osd?.notifications === true ? ["osd", "notification_width"] : null),
            default: 420,
            min: 200,
            max: 700,
            step: 10,
            unit: "px",
          },
          { kind: "custom", editor: "osdPlace:notification" },
        ],
      },
    ],
  },
  {
    id: "presets",
    label: "Presets",
    icon: 0xe728,
    sections: [
      {
        title: "Presets",
        help: "A preset is a saved copy of all your settings. Applying one replaces the current setup; the replaced setup is kept as config.before-preset.toml in the config folder.",
        fields: [{ kind: "custom", editor: "presets" }],
      },
    ],
  },
  {
    id: "about",
    label: "About",
    icon: 0xe946,
    sections: [
      {
        title: "Updates",
        help: "New versions come from Kasumi's GitHub releases.",
        fields: [
          { kind: "custom", editor: "updates" },
          {
            kind: "toggle",
            label: "Check for updates when Kasumi starts",
            help: "The notch says \"Update is available\" when there's a new version.",
            path: ["general", "check_updates"],
            default: true,
          },
          {
            kind: "toggle",
            label: "Update automatically",
            help: "Checks now and then and installs a new version by itself (Kasumi restarts).",
            path: ["general", "auto_update"],
            default: false,
          },
        ],
      },
      {
        title: "Files",
        fields: [
          { kind: "action", label: "Config file", help: "Everything here is stored in config.toml.", button: "Open", run: "open_config" },
          { kind: "action", label: "Config folder", button: "Open", run: "open_config_folder" },
          { kind: "action", label: "Log", help: "Useful when something misbehaves.", button: "Open", run: "open_log" },
        ],
      },
      {
        title: "Reset",
        fields: [
          {
            kind: "action",
            label: "Reset to defaults",
            help: "Replaces config.toml with the built-in defaults.",
            button: "Reset",
            run: "reset",
            confirm: "Click again to reset",
          },
        ],
      },
    ],
  },
];
