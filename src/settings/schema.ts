// The settings window is generated from this declaration: every setting is
// listed once, with where it lives in config.toml, its control and default.
// Adding a feature's settings = adding entries here.

import { ownsLayout, scope, WIDGETS } from "./editors";

export type Path = (string | number)[];
export type Cfg = Record<string, any>;
/** A fixed path, or one computed from the config (null hides the field). */
export type PathSpec = Path | ((cfg: Cfg) => Path | null);

interface Base {
  label: string;
  help?: string;
  path: PathSpec;
  default: unknown;
  /** Expert: folded under the card's More (set by EXPERT below). */
  more?: boolean;
  /** A small tag by its name: what it changes or needs (set by TAGS below). */
  tag?: string;
  /** Under its name instead of the help (which goes to its tooltip): Home's cards. */
  short?: string;
}

/** Animation effects per widget (Settings > Animation > Per widget): the ones that apply to each. */
const PANEL_FX = "cascade";
const ANIM_WIDGETS: [string, string, number, string][] = [
  ["clock", "Clock", 0xe823, `roll,${PANEL_FX}`],
  ["weather", "Weather", 0xe706, `roll,${PANEL_FX}`],
  ["temps", "Performance", 0xe9ca, `roll,${PANEL_FX}`],
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
/** Widgets that can highlight what's under the cursor in the bar ([widgets.<w>] hover_highlight). */
const HOVER_WIDGETS = ["tray", "buttons", "media", "tasks"];

const APPEARANCE_WIDGETS: [string, string, number, boolean][] = [
  ["clock", "Clock and calendar", 0xe823, true],
  ["tasks", "Tasks", 0xe737, true],
  ["weather", "Weather", 0xe706, true],
  ["temps", "Performance", 0xe9ca, true],
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
  /** `values`: what off and on write, when the setting isn't a true / false one. */
  | (Base & { kind: "toggle"; values?: [unknown, unknown] })
  | (Base & { kind: "slider"; min: number; max: number; step: number; unit?: string })
  | (Base & { kind: "number"; min?: number; max?: number; step?: number; unit?: string })
  | (Base & { kind: "text"; placeholder?: string })
  | (Base & { kind: "select"; options: { value: string; label: string }[] })
  | (Base & { kind: "color" })
  | (Base & { kind: "list"; placeholder?: string })
  | { kind: "autostart"; label: string; help?: string; more?: boolean; tag?: string }
  | { kind: "action"; label: string; help?: string; button: string; run: string; confirm?: string; more?: boolean; tag?: string }
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
  /** A widget page's tab it's on (Content, Look, Motion); none: above the tabs. */
  tab?: string;
  /** A card pointing to another page (its settings live there): [button, page]. */
  link?: [string, string];
  /** Why its settings don't show (when none applies): a quiet line in their place. */
  note?: (cfg: Cfg) => string | null;
  /** Its settings as cards side by side (Home), its title over them. */
  tiles?: boolean;
  /** Settings of one bar: drawn once for each bar (scope.surface set to it), consecutive
   *  ones together in the bar's card. */
  perBar?: boolean;
}

export interface Page {
  id: string;
  label: string;
  /** Segoe Fluent Icons codepoint. */
  icon: number;
  sections: Section[];
}

/** The settings as they were first written down, page by page: the places below are
 *  put together from their sections. */
const sources: Page[] = [
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
            label: "Other apps' tooltips too (experimental)",
            help: "Hover hints in File Explorer, the tray and classic Windows apps show in Kasumi's look (newer apps and browsers draw their own, which stay as they are).",
            path: (cfg) => (cfg.general?.tooltips === false ? null : ["general", "app_tooltips"]),
            default: false,
          },
          {
            kind: "toggle",
            label: "Hide the Windows taskbar",
            help: "Hidden while Kasumi runs and restored when it quits. The Win key and Start menu keep working. Turn off Windhawk mods that show the taskbar (e.g. “Taskbar auto-hide when maximized”).",
            path: ["shell", "hide_taskbar"],
            default: false,
          },
          { kind: "custom", editor: "taskbarGuide" },
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
              { value: "launcher", label: "Launcher (opens the launcher)" },
              { value: "notches", label: "Notches (shows the notches for 2 seconds; press again for the launcher)" },
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
        fields: [
          { kind: "custom", editor: "clockFormat" },
          {
            kind: "select",
            label: "Calendar look",
            help: "What the clock's panel shows. This week shows the month too when you click its days.",
            path: ["widgets", "clock", "style"],
            default: "",
            options: [
              { value: "", label: "Month" },
              { value: "week", label: "This week (today large, the week as a strip)" },
            ],
          },
          {
            kind: "toggle",
            label: "Day and year progress",
            help: "Under the week: how far through today and this year you are.",
            path: (cfg) => (cfg.widgets?.clock?.style === "week" ? ["widgets", "clock", "progress"] : null),
            default: true,
          },
          {
            kind: "select",
            label: "Month name",
            path: (cfg) => (cfg.widgets?.clock?.style === "week" ? null : ["widgets", "clock", "title_align"]),
            default: "center",
            options: [
              { value: "left", label: "Left" },
              { value: "center", label: "Centre" },
              { value: "right", label: "Right" },
              { value: "big", label: "Big (across the whole calendar)" },
            ],
          },
          {
            kind: "slider",
            label: "Month name size",
            path: (cfg) => (cfg.widgets?.clock?.style === "week" || cfg.widgets?.clock?.title_align === "big" ? null : ["widgets", "clock", "title_size"]),
            default: 13,
            min: 11,
            max: 32,
            step: 1,
            unit: "px",
          },
          {
            kind: "toggle",
            label: "Year after the month",
            help: "Another year's month always shows its year.",
            path: (cfg) => (cfg.widgets?.clock?.style === "week" || cfg.widgets?.clock?.title_align === "big" ? null : ["widgets", "clock", "show_year"]),
            default: true,
          },
          {
            kind: "toggle",
            label: "Month arrows",
            help: "Arrows beside the month's name to look at other months.",
            path: (cfg) => (cfg.widgets?.clock?.style === "week" ? null : ["widgets", "clock", "month_arrows"]),
            default: false,
          },
          { kind: "toggle", label: "Week numbers", path: ["widgets", "clock", "week_numbers"], default: true },
          { kind: "toggle", label: "Day names", help: "The Mo Tu We… row over the days.", path: ["widgets", "clock", "day_names"], default: true },
          { kind: "toggle", label: "Days of the months either side", help: "The end of last month and the start of next, faded.", path: ["widgets", "clock", "other_days"], default: true },
          { kind: "toggle", label: "Fade past days", path: ["widgets", "clock", "fade_past"], default: false },
          { kind: "toggle", label: "Dim weekends", path: ["widgets", "clock", "dim_weekends"], default: false },
          {
            kind: "select",
            label: "Today",
            path: ["widgets", "clock", "today_mark"],
            default: "pill",
            options: [
              { value: "pill", label: "Filled (in your accent)" },
              { value: "underline", label: "Underlined (in your accent)" },
            ],
          },
          {
            kind: "toggle",
            label: "Timers",
            help: "Under the calendar: start a countdown (1 min to 1 hour) or a stopwatch. While one runs, the clock shows its time; when a countdown ends, a pop-up and a sound.",
            path: ["widgets", "clock", "timers"],
            default: false,
          },
          {
            kind: "toggle",
            label: "Screen time",
            help: "Under the calendar: how long you've had each app in front today (time away from the PC isn't counted). Kept only on this PC.",
            path: ["widgets", "clock", "screen_time"],
            default: false,
          },
          { kind: "custom", editor: "worldClocks" },
        ],
      },
      {
        title: "Performance",
        collapsible: true,
        icon: 0xe9ca,
        help: "CPU, graphics and memory load from Windows; temperatures from LibreHardwareMonitor if you use it.",
        fields: [
          {
            kind: "toggle",
            label: "Temperatures from LibreHardwareMonitor",
            help: "Also temperatures, power and clocks, read from LibreHardwareMonitor (a separate free app). Off: load and memory only.",
            path: ["widgets", "temps", "lhm"],
            default: true,
          },
          {
            kind: "text",
            label: "LibreHardwareMonitor address",
            help: "LibreHardwareMonitor > Options > Remote Web Server.",
            path: (cfg) => (cfg.widgets?.temps?.lhm === false ? null : ["widgets", "temps", "url"]),
            default: "http://localhost:8085/data.json",
          },
          {
            kind: "toggle",
            label: "Apps using the most",
            help: "Under the readings in its panel: the five busiest apps, each with End (click twice) for one that's stuck.",
            path: ["widgets", "temps", "top_apps"],
            default: true,
          },
          { kind: "slider", label: "Warning temperature", path: (cfg) => (cfg.widgets?.temps?.lhm === false ? null : ["widgets", "temps", "warn"]), default: 90, min: 50, max: 105, step: 1, unit: "°" },
          { kind: "slider", label: "Update every", path: ["widgets", "temps", "interval_ms"], default: 1000, min: 250, max: 5000, step: 250, unit: "ms" },
          { kind: "custom", editor: "tempsSensors" },
          {
            kind: "select",
            label: "Look",
            help: "How its panel shows the readings.",
            path: ["widgets", "temps", "style"],
            default: "",
            options: [
              { value: "", label: "Cards (CPU and GPU side by side)" },
              { value: "dials", label: "Dials (each on a dial up to the warning)" },
              { value: "graph", label: "Graph (the last five minutes)" },
              { value: "table", label: "Table (one part per line)" },
            ],
          },
          {
            kind: "select",
            label: "Bar shows",
            path: (cfg) => (cfg.widgets?.temps?.lhm === false ? null : ["widgets", "temps", "bar_shows"]),
            default: "temps",
            options: [
              { value: "temps", label: "Temperatures" },
              { value: "load", label: "Load" },
            ],
          },
          {
            kind: "toggle",
            label: "Heat colour",
            help: "The numbers in the bar warm towards orange as they near the warning temperature.",
            path: (cfg) => (cfg.widgets?.temps?.bar_shows === "load" || cfg.widgets?.temps?.lhm === false ? null : ["widgets", "temps", "heat_colour"]),
            default: false,
          },
          {
            kind: "toggle",
            label: "Sparkline in the bar",
            help: "The last minute as a small line beside each temperature.",
            path: (cfg) => (cfg.widgets?.temps?.bar_shows === "load" || cfg.widgets?.temps?.lhm === false ? null : ["widgets", "temps", "sparkline"]),
            default: false,
          },
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
            label: "Look",
            help: "How its panel shows the weather.",
            path: ["widgets", "weather", "style"],
            default: "",
            options: [
              { value: "", label: "Classic (the conditions and the next hours or days)" },
              { value: "sky", label: "Sky (coloured like the sky outside; what is coming)" },
              { value: "curve", label: "Curve (the next 12 hours as a line)" },
              { value: "week", label: "Week (each day low to high on one scale)" },
            ],
          },
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
            path: (cfg) => (["curve", "week"].includes(cfg.widgets?.weather?.style) ? null : ["widgets", "weather", "forecast"]),
            default: "hourly",
            options: [
              { value: "hourly", label: "The next hours" },
              { value: "daily", label: "The next days (high and low)" },
            ],
          },
          {
            kind: "slider",
            label: "Hours in the panel",
            path: (cfg) => (cfg.widgets?.weather?.forecast === "daily" || ["curve", "week"].includes(cfg.widgets?.weather?.style) ? null : ["widgets", "weather", "hours"]),
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
            label: "Look",
            help: "How the song shows in its panel.",
            path: ["widgets", "media", "style"],
            default: "",
            options: [
              { value: "", label: "Classic (cover, seek bar, keys)" },
              { value: "record", label: "Record (the cover turns while it plays; a ring to seek)" },
              { value: "glow", label: "Glow (the cover's colours glow behind big type)" },
              { value: "cover", label: "Cover (the cover across the panel, fading into the bar)" },
            ],
          },
          {
            kind: "select",
            label: "Before the title",
            help: "Something small before the song's title in the bar, whatever the panel's look.",
            path: ["widgets", "media", "mark"],
            default: "",
            options: [
              { value: "", label: "Nothing" },
              { value: "record", label: "Record (a little record, turning while it plays)" },
              { value: "bars", label: "Bars (bars that move while it plays)" },
              { value: "cover", label: "Cover (the cover, small)" },
            ],
          },
          {
            kind: "select",
            label: "Playback keys",
            help: "Previous / play-pause / next beside the title. They're always in the media panel (click the title).",
            path: ["widgets", "media", "keys"],
            default: "off",
            options: [
              { value: "off", label: "Off" },
              { value: "always", label: "Always" },
              { value: "open", label: "Bar open (only while the bar is open)" },
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
        help: "For the launcher however it opens: from its search icon on a bar, the Windows key or its hotkey.",
        fields: [
          {
            kind: "select",
            label: "Look",
            help: "Your buttons (user, power, your own) show in every look.",
            path: ["widgets", "launcher", "style"],
            default: "",
            options: [
              { value: "", label: "List (out of the dock or bar)" },
              { value: "centre", label: "Centre (a box in the middle of the screen)" },
              { value: "grid", label: "Grid (your Start pins as icons before you type)" },
              { value: "prompt", label: "Prompt (one line; Tab completes the best match)" },
            ],
          },
          {
            kind: "toggle",
            label: "Dim the screen",
            help: "The screen behind the centred launcher darkens a little.",
            path: (cfg) => (cfg.widgets?.launcher?.style === "centre" ? ["widgets", "launcher", "dim"] : null),
            default: true,
          },
          { kind: "slider", label: "Results shown", help: "How many apps the search lists at most.", path: ["widgets", "launcher", "results"], default: 8, min: 3, max: 20, step: 1 },
          { kind: "slider", label: "Width", path: ["widgets", "launcher", "panel_width"], default: 340, min: 260, max: 600, step: 10, unit: "px" },
          {
            kind: "toggle",
            label: "Calculator",
            help: "Type a sum (24*7, (3+4)^2, sqrt(2)) and its answer shows first; Enter copies it.",
            path: ["widgets", "launcher", "calculator"],
            default: false,
          },
          {
            kind: "toggle",
            label: "Settings pages",
            help: "Windows Settings pages show among the results: type bluetooth, wallpaper, mouse, dark mode...",
            path: ["widgets", "launcher", "settings"],
            default: false,
          },
          {
            kind: "toggle",
            label: "Open windows",
            help: "Your open windows show among the results, found by their title or their app's name; Enter switches to one.",
            path: ["widgets", "launcher", "windows"],
            default: false,
          },
          {
            kind: "toggle",
            label: "Pinned apps",
            help: "Before you type, the launcher lists the apps pinned to Windows' Start menu (as many as Results shown).",
            path: ["widgets", "launcher", "pinned"],
            default: false,
          },
          {
            kind: "toggle",
            label: "Clipboard history",
            help: "Type a comma (,) to search what you've copied; Enter pastes it into the app you were in. Kasumi keeps the history (in memory only) while this is on.",
            path: ["widgets", "launcher", "clipboard"],
            default: false,
          },
          { kind: "toggle", label: "User button", help: "Your account picture and name; opens Settings › Your info.", path: ["widgets", "launcher", "user"], default: false },
          { kind: "custom", editor: "launcherPower" },
          { kind: "custom", editor: "launcherButtons" },
          {
            kind: "select",
            label: "Buttons and user",
            help: "Where the user, your buttons and the power buttons go.",
            path: ["widgets", "launcher", "buttons_position"],
            default: "row",
            options: [
              { value: "row", label: "Row (in a row below the apps)" },
              { value: "left", label: "Column (in a column on the left)" },
            ],
          },
          {
            kind: "select",
            label: "Buttons position",
            help: "In the row: which side the buttons sit on.",
            path: (cfg) => {
              const l = cfg.widgets?.launcher;
              const some = (l?.power ?? []).length || (l?.buttons ?? []).length;
              return some && l?.buttons_position !== "left" ? ["widgets", "launcher", "power_align"] : null;
            },
            default: "right",
            options: [
              { value: "left", label: "Left" },
              { value: "center", label: "Middle" },
              { value: "right", label: "Right" },
            ],
          },
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
              { value: "title", label: "Title (the window's title)" },
              { value: "app", label: "App (its name)" },
              { value: "both", label: "Both (app name — window title)" },
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
          {
            kind: "select",
            label: "Look",
            help: "What a click shows: This PC, or a panel of your drives.",
            path: ["widgets", "disk", "style"],
            default: "",
            options: [
              { value: "", label: "This PC (a click opens it)" },
              { value: "drives", label: "Drives (free space, the Recycle Bin, Disk Cleanup)" },
              { value: "rings", label: "Rings (a big ring per drive)" },
            ],
          },
          { kind: "slider", label: "Nearly full at", help: "Drives this full turn orange.", path: ["widgets", "disk", "warn"], default: 90, min: 50, max: 99, step: 1, unit: "%" },
          {
            kind: "toggle",
            label: "Only when nearly full",
            help: "Stays out of the bar until a drive is nearly full.",
            path: ["widgets", "disk", "only_when_full"],
            default: false,
          },
        ],
      },
      {
        title: "Privacy",
        collapsible: true,
        icon: 0xe720,
        help: "Shows while an app uses your microphone, camera or location; click for which apps.",
        fields: [
          { kind: "toggle", label: "Microphone", path: ["widgets", "privacy", "mic"], default: true },
          { kind: "toggle", label: "Camera", path: ["widgets", "privacy", "camera"], default: true },
          { kind: "toggle", label: "Location", path: ["widgets", "privacy", "location"], default: true },
          {
            kind: "toggle",
            label: "Switches",
            help: "In its panel: mute every microphone, and turn the camera and location off for all apps (Windows' own privacy switches). It stays in the bar as a shield, and a muted microphone shows in red.",
            path: ["widgets", "privacy", "switches"],
            default: false,
            tag: "Changes Windows",
          },
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
              { value: "open", label: "App (opens the app that sent it)" },
              { value: "nothing", label: "Nothing (does nothing)" },
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
          {
            kind: "select",
            label: "Look",
            help: "How its panel shows your copies.",
            path: ["widgets", "clipboard", "style"],
            default: "",
            options: [
              { value: "", label: "List (a row per copy)" },
              { value: "tiles", label: "Tiles (colours, links and pictures shown as such)" },
              { value: "compact", label: "Compact (one line each, a search box, 1 to 9)" },
            ],
          },
          { kind: "toggle", label: "Images", help: "Also keep copied images (up to 8).", path: ["widgets", "clipboard", "images"], default: true },
          { kind: "toggle", label: "Item count", path: ["widgets", "clipboard", "count"], default: false },
          {
            kind: "toggle",
            label: "Screenshots",
            help: "Your newest three screenshots at the top of its panel: click one to copy it (paste it into a chat or a document), or Snip to take one.",
            path: ["widgets", "clipboard", "screenshots"],
            default: false,
          },
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
            kind: "toggle",
            label: "Pace mark",
            help: "A mark across the meter for how much of the time has gone: usage past it means you're going faster than the limit allows.",
            path: (cfg) => ((cfg.widgets?.claude?.indicator ?? "bar") === "bar" ? ["widgets", "claude", "pace_mark"] : null),
            default: false,
          },
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
          {
            kind: "select",
            label: "Look",
            help: "How its panel shows your usage.",
            path: ["widgets", "claude", "style"],
            default: "",
            options: [
              { value: "", label: "Report (everything as lines)" },
              { value: "rings", label: "Rings (the two limits, when they reset)" },
              { value: "pace", label: "Pace (whether you'll run out before the reset)" },
              { value: "sessions", label: "Sessions (working, waiting for you, idle)" },
            ],
          },
          { kind: "toggle", label: "Last 7 days in the panel", path: (cfg) => (cfg.widgets?.claude?.style ? null : ["widgets", "claude", "weekly"]), default: true },
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
          {
            kind: "select",
            label: "Look",
            help: "How its panel lays out. The wheel over a slider moves it in every look.",
            path: ["widgets", "quick", "style"],
            default: "",
            options: [
              { value: "", label: "Classic (sliders side by side, a row of tiles)" },
              { value: "blocks", label: "Blocks (two-line tiles, thick sliders)" },
              { value: "rings", label: "Rings (volume, brightness and battery as rings; round buttons)" },
              { value: "list", label: "List (a row per setting, switches at the end)" },
            ],
          },
          {
            kind: "select",
            label: "Battery shows",
            help: "Beside its icon in the bar.",
            path: (cfg) => (["auto", "battery", undefined].includes(cfg.widgets?.quick?.icon) ? ["widgets", "quick", "battery_style"] : null),
            default: "percent",
            options: [
              { value: "percent", label: "Its percentage" },
              { value: "bar", label: "A bar" },
            ],
          },
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
          {
            kind: "toggle",
            label: "Device batteries",
            help: "A line under battery and network: your Bluetooth mice, keyboards and headphones, and how charged they are.",
            path: ["widgets", "quick", "devices"],
            default: false,
          },
          {
            kind: "toggle",
            label: "Wi-Fi and Bluetooth tiles switch",
            help: "Their tiles turn Wi-Fi and Bluetooth on and off (lit while on) instead of opening Settings. The Dark mode tile always switches.",
            path: ["widgets", "quick", "switches"],
            default: false,
          },
        ],
      },
      {
        title: "Buttons",
        collapsible: true,
        icon: 0xe7e8,
        fields: [{ kind: "custom", editor: "buttonsWidgets" }],
      },
      {
        title: "Batteries",
        collapsible: true,
        icon: 0xe83f,
        help: "This PC's battery (laptops) and your Bluetooth devices' (mice, keyboards, headphones). Click it for the list.",
        fields: [
          {
            kind: "select",
            label: "Shows",
            path: ["widgets", "batteries", "show"],
            default: "lowest",
            options: [
              { value: "lowest", label: "Lowest (the lowest battery)" },
              { value: "all", label: "All (every battery)" },
              { value: "chosen", label: "Picked (the ones you pick)" },
            ],
          },
          { kind: "custom", editor: "batteryPicks" },
          {
            kind: "toggle",
            label: "This PC's battery",
            help: "Counted in the bar too (laptops).",
            path: (cfg) => (cfg.widgets?.batteries?.show === "chosen" ? null : ["widgets", "batteries", "laptop"]),
            default: true,
          },
          { kind: "toggle", label: "Device names", help: "Each battery's name beside its level in the bar.", path: ["widgets", "batteries", "names"], default: false },
          { kind: "custom", editor: "batteryNames" },
          {
            kind: "select",
            label: "Level as",
            help: "In the bar: the number, a small bar, or just the battery icon.",
            path: ["widgets", "batteries", "level_as"],
            default: "percent",
            options: [
              { value: "percent", label: "Percent" },
              { value: "bar", label: "Bar" },
              { value: "icon", label: "Icon" },
            ],
          },
          {
            kind: "select",
            label: "Device icons",
            help: "A mouse, headphones or a controller (known from the device's name) instead of the battery or Bluetooth sign.",
            path: ["widgets", "batteries", "device_icons"],
            default: "bluetooth",
            options: [
              { value: "bluetooth", label: "Battery" },
              { value: "kind", label: "By kind" },
            ],
          },
          { kind: "toggle", label: "Time left", help: "The laptop's time left beside its level in the bar.", path: ["widgets", "batteries", "time_left"], default: false },
          { kind: "slider", label: "Low at", help: "At or under this a battery turns orange.", path: ["widgets", "batteries", "low_at"], default: 20, min: 5, max: 50, step: 5, unit: "%" },
          { kind: "toggle", label: "Only when low", help: "Stays out of the bar until a battery is low.", path: ["widgets", "batteries", "only_when_low"], default: false },
          { kind: "toggle", label: "Warn me", help: "A pop-up from the notch once, when a battery gets low.", path: ["widgets", "batteries", "low_warning"], default: false },
          {
            kind: "toggle",
            label: "Keep disconnected devices",
            help: "In the panel, greyed: their last level and when they were last seen.",
            path: ["widgets", "batteries", "keep_disconnected"],
            default: false,
          },
          {
            kind: "select",
            label: "Tell me when charged",
            help: "For a laptop kept plugged in: a pop-up when it reaches the level, so you can unplug it.",
            path: ["widgets", "batteries", "charged_at"],
            default: "off",
            options: [
              { value: "off", label: "Off" },
              { value: "80", label: "80%" },
              { value: "100", label: "100%" },
            ],
          },
        ],
      },
      {
        title: "Tasks",
        collapsible: true,
        icon: 0xe737,
        help: "Open apps. Click: switch to it (again: minimize). Middle click: close. Right click: its windows, new window, close all.",
        fields: [
          {
            kind: "select",
            label: "Look",
            help: "Window previews on hover are under 'Hovering an app with several windows'.",
            path: ["widgets", "tasks", "style"],
            default: "",
            options: [
              { value: "", label: "Icons (dots for windows, a line under the app in front)" },
              { value: "labels", label: "Labels (each app a pill with its name and window count)" },
              { value: "dots", label: "Dots (a mark per app while the dock rests; icons as it opens)" },
            ],
          },
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
            help: "Monochrome keeps the detail in grey; Accent paints each icon's shape in the accent colour (Look › Colours).",
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
            path: (cfg) => (["labels", "dots"].includes(cfg.widgets?.tasks?.style) ? null : ["widgets", "tasks", "labels"]),
            default: "none",
            options: [
              { value: "none", label: "Icons only" },
              { value: "active", label: "Title of the window in front" },
            ],
          },
          {
            kind: "toggle",
            label: "Show apps asking for attention",
            help: "A flashing accent dot on an app that wants you (a message, a finished task), until you switch to it.",
            path: ["widgets", "tasks", "attention"],
            default: true,
          },
          {
            kind: "select",
            label: "Hovering an app with several windows",
            path: ["widgets", "tasks", "hover"],
            default: "list",
            options: [
              { value: "list", label: "List (lists its windows)" },
              { value: "previews", label: "Previews (shows previews of its windows)" },
            ],
          },
          {
            kind: "select",
            label: "Scrolling over an app",
            path: ["widgets", "tasks", "scroll"],
            default: "windows",
            options: [
              { value: "windows", label: "Windows (cycles its windows)" },
              { value: "volume", label: "Volume (changes its volume)" },
              { value: "off", label: "Nothing (does nothing)" },
            ],
          },
          {
            kind: "toggle",
            label: "Win + 1…9 opens apps",
            help: "Like the Windows taskbar: Win + 3 switches to (or starts) the third app here.",
            path: ["widgets", "tasks", "win_keys"],
            default: true,
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
            label: "Look",
            help: "How the icons behind the arrow show.",
            path: ["widgets", "tray", "style"],
            default: "",
            options: [
              { value: "", label: "Grid (a panel of icons)" },
              { value: "list", label: "List (each with its app's name)" },
              { value: "slide", label: "Slide (they slide out along the bar)" },
            ],
          },
          {
            kind: "select",
            label: "Show",
            path: (cfg) => (cfg.widgets?.tray?.style === "slide" ? null : ["widgets", "tray", "mode"]),
            default: "collapsed",
            options: [
              { value: "collapsed", label: "Collapsed (a button with the icons in a panel)" },
              { value: "expanded", label: "Expanded (every icon in the bar)" },
            ],
          },
          {
            kind: "list",
            label: "Kept in the bar",
            help: "Apps whose icons always sit in the bar, before the arrow: their program names, comma separated (Discord.exe, Spotify.exe). The arrow then says how many are behind it.",
            path: (cfg) => (cfg.widgets?.tray?.mode === "expanded" && cfg.widgets?.tray?.style !== "slide" ? null : ["widgets", "tray", "pinned"]),
            default: [],
            placeholder: "Discord.exe, Spotify.exe",
          },
          {
            kind: "toggle",
            label: "Dot when one changes",
            help: "A dot on the arrow when an icon behind it changes (a new message, an update), until you open it.",
            path: (cfg) => (cfg.widgets?.tray?.mode === "expanded" && cfg.widgets?.tray?.style !== "slide" ? null : ["widgets", "tray", "activity_dot"]),
            default: false,
          },
          {
            kind: "select",
            label: "Icons",
            help: "Monochrome keeps the detail in grey; Accent paints each icon's shape in the accent colour (Look › Colours).",
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
            path: (cfg) => (cfg.widgets?.tray?.mode === "expanded" || ["list", "slide"].includes(cfg.widgets?.tray?.style) ? null : ["widgets", "tray", "icons_per_row"]),
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
        fields: [
          {
            kind: "select",
            label: "Look",
            help: "Your items stay the same in every look.",
            path: ["widgets", "menu", "style"],
            default: "",
            options: [
              { value: "", label: "List (icons and names, lines between groups)" },
              { value: "grid", label: "Grid (places as tiles, power actions as a row under them)" },
              { value: "strip", label: "Strip (icons in one row, the name under the pointer)" },
              { value: "greeting", label: "Greeting (a hello, the date and time on, then the list with shortcuts)" },
            ],
          },
          { kind: "custom", editor: "menuItems" },
        ],
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
            kind: "select",
            label: "Background",
            help: "Acrylic blurs what's behind the bars, Mica your wallpaper (Windows 11). Both need Windows' transparency effects on.",
            path: ["theme", "backdrop"],
            default: "solid",
            options: [
              { value: "solid", label: "Solid" },
              { value: "acrylic", label: "Acrylic" },
              { value: "mica", label: "Mica" },
            ],
          },
          {
            kind: "slider",
            label: "Tint",
            help: "How much of the background colour lies over the blur.",
            path: (cfg) => ((cfg.theme?.backdrop ?? "solid") === "solid" ? null : ["theme", "backdrop_tint"]),
            default: 60,
            min: 0,
            max: 100,
            step: 5,
            unit: "%",
          },
          {
            kind: "slider",
            label: "Opacity",
            help: "Of the bars, notches and panels; text and icons stay solid.",
            path: (cfg) => ((cfg.theme?.backdrop ?? "solid") === "solid" ? ["theme", "opacity"] : null),
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
            help: "Highlights in panels (the row under the cursor), the line under the app in front, sliders, bars and key readings in the accent colour. Each widget can differ on its own page (Widgets › its Look tab).",
            path: ["theme", "accent_details"],
            default: false,
          },
          {
            kind: "toggle",
            label: "Hover highlights",
            help: "A soft box behind what's under the cursor in the bars: tray icons, buttons, media keys, the dock's apps. Each widget can differ on its own page (Widgets › its Look tab).",
            path: ["theme", "hover_highlights"],
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
          ...(HOVER_WIDGETS.includes(w) ? [{ kind: "custom" as const, editor: `hoverWidget:${w}` }] : []),
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
              { value: "drop", label: "Drop (from the top edge)" },
              { value: "spread", label: "Spread (out of the notches)" },
            ],
          },
          {
            kind: "select",
            label: "Bar content",
            help: "How the open bar's widgets show while it opens.",
            path: ["animation", "bar_content"],
            default: "slide",
            options: [
              { value: "slide", label: "Slide (in with the bar)" },
              { value: "reveal", label: "Uncover (stay in place, the bar uncovers them)" },
              { value: "fade", label: "Fade (stay in place and fade in)" },
            ],
          },
          {
            kind: "select",
            label: "Panel content",
            help: "How a widget's panel (calendar, weather...) shows its content while it opens.",
            path: ["animation", "panel_content"],
            default: "reveal",
            options: [
              { value: "reveal", label: "Uncover (stays in place, the panel uncovers it)" },
              { value: "fade", label: "Fade (stays in place and fades in)" },
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
        help: "For every widget's panel and numbers; each widget can differ on its own page (Widgets › its Motion tab).",
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
        title: "Context menu",
        help: "Kasumi's own right-click menu (Shell › Context menus).",
        fields: [
          {
            kind: "select",
            label: "Opening",
            path: ["animation", "menu_open"],
            default: "expand",
            options: [
              { value: "expand", label: "Grow (out of where it opens; submenus sideways)" },
              { value: "fade", label: "Fade (fades in)" },
              { value: "none", label: "None (no animation)" },
            ],
          },
          { kind: "toggle", label: "Entries one by one", help: "Its entries come in one after another as it opens.", path: ["animation", "menu_cascade"], default: true },
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
          { kind: "slider", label: "Context menu stiffness", path: ["animation", "menu_stiffness"], default: 775, min: 200, max: 6000, step: 20 },
          { kind: "slider", label: "Context menu damping", path: ["animation", "menu_damping"], default: 43, min: 10, max: 160, step: 1 },
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
          {
            kind: "select",
            label: "While the bar is open",
            help: "Where pop-ups show when the bar is open (native Kasumi; the web version always drops them below).",
            path: ["osd", "open_bar"],
            default: "inline",
            options: [
              { value: "inline", label: "In the bar (in place of their notch's widgets)" },
              { value: "below", label: "Pill (in a pill below the bar)" },
            ],
          },
          { kind: "slider", label: "Pill width", path: ["osd", "width"], default: 230, min: 160, max: 400, step: 5, unit: "px" },
          { kind: "slider", label: "Pill height", help: "0: as tall as the open bar.", path: ["osd", "height"], default: 0, min: 0, max: 48, step: 1, unit: "px" },
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
          {
            kind: "toggle",
            label: "Hide the Windows brightness flyout",
            help: "While Kasumi shows brightness, the Windows pop-up at the bottom of the screen doesn't.",
            path: ["osd", "hide_windows_brightness"],
            default: true,
          },
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
        title: "Files and downloads",
        collapsible: true,
        group: "Pop-ups",
        icon: 0xe8c8,
        fields: [
          {
            kind: "toggle",
            label: "Show file copy progress",
            help: "While File Explorer copies or moves files, their progress shows as a pop-up. (Downloads and installers keep theirs in their own windows.)",
            path: ["osd", "file_progress"],
            default: false,
          },
          {
            kind: "toggle",
            label: "Finished downloads",
            help: "A pop-up when a download lands in your Downloads folder, with its name.",
            path: ["osd", "downloads"],
            default: false,
          },
        ],
      },
      {
        title: "Scrolling",
        collapsible: true,
        group: "Pop-ups",
        icon: 0xe8cb,
        fields: [
          {
            kind: "toggle",
            label: "Show scroll position",
            help: "A scroll bar at the right edge of the screen while a folder or app under the mouse scrolls. Browsers and apps built on them aren't included.",
            path: ["osd", "scroll"],
            default: false,
          },
          {
            kind: "slider",
            label: "Length",
            path: (cfg) => (cfg.osd?.scroll ? ["osd", "scroll_length"] : null),
            default: 800,
            min: 300,
            max: 1400,
            step: 20,
            unit: "px",
          },
          {
            kind: "slider",
            label: "Width",
            help: "How far its notch stands out of the screen's edge.",
            path: (cfg) => (cfg.osd?.scroll ? ["osd", "scroll_width"] : null),
            default: 10,
            min: 6,
            max: 30,
            step: 1,
            unit: "px",
          },
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
              { value: "line", label: "One line (app, title and text)" },
              { value: "two", label: "Two lines (title over the text)" },
              { value: "compact", label: "Icon (just the app's icon and name)" },
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
            kind: "select",
            label: "Clicking it",
            help: "Native Kasumi.",
            path: (cfg) => (cfg.osd?.notifications === true ? ["osd", "notification_click"] : null),
            default: "app",
            options: [
              { value: "app", label: "App (opens the app that sent it)" },
              { value: "panel", label: "Kasumi (opens Kasumi's notifications panel)" },
              { value: "center", label: "Windows (opens Windows' notification center)" },
              { value: "dismiss", label: "Dismiss (dismisses it)" },
            ],
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
    id: "context-menu",
    label: "Context menu",
    icon: 0xe700,
    sections: [
      {
        title: "Context menu (experimental)",
        help: "Kasumi draws the right-click menu in File Explorer and on the desktop, in its own look: Windows' entries (and other apps'), sized to fit, highlighted in the accent colour. It's new and some things may not work yet: Explorer's navigation pane and other apps' Open / Save dialogs keep Windows' menu, and right-dragging files doesn't work while it's on. Native Kasumi only.",
        fields: [
          { kind: "toggle", label: "Kasumi's context menu", help: "Off: Windows' menu (or your own, like Nilesoft Shell), as usual.", path: ["context_menu", "enabled"], default: false },
          { kind: "toggle", label: "Icons", help: "An icon beside each entry that has one.", path: ["context_menu", "icons"], default: false },
          {
            kind: "toggle",
            label: "Tray icons' menus",
            help: "Right-clicking an icon in Kasumi's tray shows the app's menu in Kasumi's look too. Menus an app draws itself (not Windows' kind) stay as they are.",
            path: (cfg) => (cfg.context_menu?.enabled ? ["context_menu", "tray_apps"] : null),
            default: true,
          },
        ],
      },
      {
        title: "Look",
        help: "Kasumi's menus (in Explorer, on the bars and in the launcher) look like the bars (Look › Colours), or have their own colours and background.",
        fields: [
          { kind: "toggle", label: "Own look", help: "Off: the same as the bars (Look › Colours).", path: ["context_menu", "own_style"], default: false },
          {
            kind: "color",
            label: "Background tint",
            path: (cfg) => (cfg.context_menu?.own_style ? ["context_menu", "style", "background"] : null),
            default: "#000000",
          },
          {
            kind: "select",
            label: "Background",
            help: "Acrylic blurs what's behind the menu, Mica your wallpaper (Windows 11).",
            path: (cfg) => (cfg.context_menu?.own_style ? ["context_menu", "style", "backdrop"] : null),
            default: "solid",
            options: [
              { value: "solid", label: "Solid" },
              { value: "acrylic", label: "Acrylic" },
              { value: "mica", label: "Mica" },
            ],
          },
          {
            kind: "slider",
            label: "Tint",
            help: "How much of the background colour lies over the blur.",
            path: (cfg) => (cfg.context_menu?.own_style && (cfg.context_menu?.style?.backdrop ?? "solid") !== "solid" ? ["context_menu", "style", "backdrop_tint"] : null),
            default: 60,
            min: 0,
            max: 100,
            step: 5,
            unit: "%",
          },
          {
            kind: "slider",
            label: "Opacity",
            help: "Solid menus stay at least 90% opaque, to read well.",
            path: (cfg) => (cfg.context_menu?.own_style && (cfg.context_menu?.style?.backdrop ?? "solid") === "solid" ? ["context_menu", "style", "opacity"] : null),
            default: 100,
            min: 90,
            max: 100,
            step: 1,
            unit: "%",
          },
          {
            kind: "color",
            label: "Text",
            path: (cfg) => (cfg.context_menu?.own_style ? ["context_menu", "style", "text"] : null),
            default: "#ffffff",
          },
          {
            kind: "color",
            label: "Highlight",
            help: "The row under the mouse.",
            path: (cfg) => (cfg.context_menu?.own_style ? ["context_menu", "style", "accent"] : null),
            default: "#ffffff",
          },
        ],
      },
      {
        title: "Entries",
        help: "Reorder, hide, rename and group the menu's entries, for each kind of click, and add your own folders and commands. Kasumi starts from a tidy layout: Pin/Unpin and More options folders, Terminal, File manage, Go to.",
        fields: [{ kind: "custom", editor: "contextMenuEditor" }],
      },
    ],
  },
  {
    id: "switchers",
    label: "Switchers",
    icon: 0xe7c4,
    sections: [
      {
        title: "Switchers",
        help: "Kasumi's own pop-ups for switching windows and keyboard languages, in place of Windows' ones. Native Kasumi only (Kasumi › Updates and files).",
        fields: [
          {
            kind: "select",
            label: "Position",
            help: "Where they show (a switcher can have its own below). Without a bar there (no dock on that monitor, say), in the middle.",
            path: ["alt_tab", "position"],
            default: "top",
            options: [
              { value: "top", label: "Top bar (out of the top bar's notch)" },
              { value: "center", label: "Middle (in the middle of the screen)" },
              { value: "bottom", label: "Dock (out of the dock's notch)" },
            ],
          },
        ],
      },
      {
        title: "Alt + Tab",
        help: "Switch windows. Tab / Shift + Tab or the arrow keys move, letting go of Alt switches, Esc cancels.",
        fields: [
          { kind: "toggle", label: "Kasumi's Alt + Tab", help: "Off: the Windows switcher, as usual.", path: ["alt_tab", "enabled"], default: true },
          {
            kind: "toggle",
            label: "Replace the tasks widget",
            help: "When that notch holds the tasks widget, the notch turns into the switcher (its apps step aside) instead of the switcher hanging under it.",
            path: ["alt_tab", "replace_tasks"],
            default: true,
          },
          {
            kind: "select",
            label: "Windows from",
            path: ["alt_tab", "monitors"],
            default: "current",
            options: [
              { value: "current", label: "This monitor (the one you're working on)" },
              { value: "all", label: "All monitors" },
            ],
          },
          { kind: "toggle", label: "Previews", help: "A live preview of each window.", path: ["alt_tab", "previews"], default: true },
          {
            kind: "select",
            label: "Under each window",
            help: "\"Nothing\" only works with previews on; without them each window shows its icon and title.",
            path: ["alt_tab", "labels"],
            default: "both",
            options: [
              { value: "both", label: "Icon and title" },
              { value: "icons", label: "Icon" },
              { value: "titles", label: "Title" },
              { value: "none", label: "Nothing" },
            ],
          },
          {
            kind: "select",
            label: "Show desktop",
            help: "A choice that shows the desktop (as Win + D). First: Alt + Tab still goes to the previous window; Shift + Tab reaches it.",
            path: ["alt_tab", "desktop"],
            default: "off",
            options: [
              { value: "off", label: "Off" },
              { value: "first", label: "First in the list" },
              { value: "last", label: "Last in the list" },
            ],
          },
        ],
      },
      {
        title: "Languages (Win + Space)",
        help: "Switch keyboard languages. Hold Win and press Space: Space / Shift + Space move, letting go of Win switches, Esc cancels.",
        fields: [
          { kind: "toggle", label: "Kasumi's language picker", help: "Off: Windows' picker, as usual.", path: ["lang_switch", "enabled"], default: true },
          {
            kind: "select",
            label: "Position",
            help: "Out of the language widget: from its notch, when there's one on that monitor (else the default).",
            path: ["lang_switch", "position"],
            default: "default",
            options: [
              { value: "default", label: "Default (the position above)" },
              { value: "widget", label: "Out of the language widget" },
              { value: "top", label: "Top bar (out of the top bar's notch)" },
              { value: "center", label: "Middle (in the middle of the screen)" },
              { value: "bottom", label: "Dock (out of the dock's notch)" },
            ],
          },
          {
            kind: "select",
            label: "Each language",
            path: ["lang_switch", "labels"],
            default: "both",
            options: [
              { value: "both", label: "Code and name" },
              { value: "codes", label: "Code" },
            ],
          },
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
          {
            kind: "toggle",
            label: "Show what's new after updates",
            help: "The first time a new version starts, a window shows its notes.",
            path: ["general", "update_notes"],
            default: true,
          },
          { kind: "action", label: "What's new", help: "This version's notes.", button: "Show", run: "notes" },
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

// --- The places ----------------------------------------------------------------------
// The sidebar holds six places; each has one or more pages, put together from the
// sections above (a section can be split by its settings' names).

const source = (page: string): Page => sources.find((p) => p.id === page)!;

/** A section of a page above, by its title. */
function sec(page: string, title: string, extra: Partial<Section> = {}): Section {
  const found = source(page).sections.find((x) => x.title === title);
  if (!found) throw new Error(`settings: no section "${title}" on ${page}`);
  return { ...found, ...extra };
}

/** Every section of a page above (that `keep` keeps). */
const all = (page: string, keep: (s: Section) => boolean = () => true): Section[] => source(page).sections.filter(keep);

/** Part of a section: the settings named (custom editors as "<editor>"), as a section of its own. */
function part(page: string, title: string, names: string[], extra: Partial<Section> & { title: string }): Section {
  const from = sec(page, title);
  const name = (f: Field) => (f.kind === "custom" ? `<${f.editor}>` : f.label);
  const fields = names.map((n) => {
    const f = from.fields.find((x) => name(x) === n);
    if (!f) throw new Error(`settings: no "${n}" in ${page} › ${title}`);
    return f;
  });
  const { collapsible, attach, icon, group, ...rest } = from;
  void collapsible, attach, icon, group;
  return { ...rest, fields, ...extra };
}

/** One setting of an old page under another name, with a short line (Home's cards). */
function pick(page: string, title: string, label: string, name: string, short: string): Field {
  const f = part(page, title, [label], { title: "" }).fields[0];
  return { ...f, label: name, short } as Field;
}

/** The widgets page's sections, by widget type (as the page titled them). */
const WIDGET_SECTION: Record<string, string> = {
  language: "Keyboard language",
  workspaces: "Workspaces (Komorebi)",
};

/** A widget's settings that are about how it looks: they go on its Look tab. */
const LOOK_SETTINGS = new Set([
  "Look",
  "Before the title",
  "Dim the screen",
  "Calendar look",
  "Day and year progress",
  "Month name",
  "Month name size",
  "Year after the month",
  "Month arrows",
  "Week numbers",
  "Day names",
  "Days of the months either side",
  "Fade past days",
  "Dim weekends",
  "Today",
  "Bar shows",
  "Heat colour",
  "Sparkline in the bar",
  "Pace mark",
  "Level as",
  "Device icons",
]);

/** A page per widget: where it is (or a way onto a bar), then its settings on three
 *  tabs: Content (what it shows and does), Look (its look, fonts, accent), Motion (its
 *  effects). The launcher's Start menu settings join its Content. */
function widgetPage(w: { id: string; label: string; icon: number }): Page {
  const title = WIDGET_SECTION[w.id] ?? w.label;
  const own = source("widget-settings").sections.find((x) => x.title === title);
  const label = (f: Field) => ("label" in f ? f.label : "");
  const look = own?.fields.filter((f) => LOOK_SETTINGS.has(label(f))) ?? [];
  const content = own?.fields.filter((f) => !LOOK_SETTINGS.has(label(f))) ?? [];
  const fonts = source("appearance").sections.find((x) => x.group === "Per widget" && x.fields.some((f) => f.kind === "custom" && f.editor === `fontPicker:${w.id}`));
  const motion = source("animation").sections.find((x) => x.group === "Per widget" && x.fields.some((f) => f.kind === "custom" && f.editor.startsWith(`animWidget:${w.id}:`)));
  const sections: Section[] = [{ title: "", fields: [{ kind: "custom", editor: `widgetHeader:${w.id}` }] }];
  if (w.id === "launcher") sections.push({ ...sec("general", "Launcher", { title: "Start menu" }), tab: "Content" });
  if (content.length) sections.push({ title: "Settings", help: own?.help, fields: content, tab: "Content" });
  if (look.length) sections.push({ title: "Its look", fields: look, tab: "Look" });
  if (fonts) sections.push({ title: "Text and accent", fields: fonts.fields, tab: "Look" });
  if (motion) sections.push({ title: "Effects", fields: motion.fields, tab: "Motion" });
  return { id: `w-${w.id}`, label: w.label, icon: w.icon, sections };
}

export const pages: Page[] = [
  {
    id: "home",
    label: "Home",
    icon: 0xe80f,
    sections: [
      { title: "Your bars", help: "Click a widget for its settings.", fields: [{ kind: "custom", editor: "homeBars" }] },
      {
        title: "",
        tiles: true,
        fields: [
          pick("general", "Behaviour", "Hide the Windows taskbar", "Hide the taskbar", ""),
          pick("appearance", "Colors", "Accent", "Accent", ""),
        ],
      },
      {
        title: "What Kasumi replaces",
        tiles: true,
        fields: [
          pick("general", "Launcher", "Replace the Start menu", "Start menu", "The Windows key opens the launcher"),
          pick("switchers", "Alt + Tab", "Kasumi's Alt + Tab", "Alt + Tab", "Kasumi's window switcher"),
          pick("switchers", "Languages (Win + Space)", "Kasumi's language picker", "Keyboard languages", "Win + Space picks the language"),
          pick("osd", "On-screen display", "Show pop-ups", "Volume and brightness", "Pop-ups from the notch"),
          pick("context-menu", "Context menu (experimental)", "Kasumi's context menu", "Context menus", "Explorer, the bars, the launcher"),
        ],
      },
      sec("general", "Startup"),
    ],
  },
  // Bars and dock
  {
    id: "layout",
    label: "Layout",
    icon: 0xe71d,
    sections: [
      sec("widgets", "Layout for", { title: "" }),
      sec("widgets", "Placement"),
      sec("widgets", "Bar", { perBar: true, badge: undefined }),
      part("widgets", "Bar settings", ["<surfaceStyle>"], { title: "Style", scope: "global", perBar: true }),
      sec("widgets", "This monitor"),
    ],
  },
  {
    id: "opening",
    label: "Opening",
    icon: 0xe8b0,
    sections: [
      sec("widgets", "Layout for", { title: "" }),
      part("widgets", "Bar settings", ["Reserve space", "Hot corner: left", "Hot corner: right", "Anywhere along the edge", "Hot corner size", "Edge zone size"], {
        title: "Hot corners and space",
        scope: "global",
        perBar: true,
        note: (cfg) => (cfg.surfaces?.[scope.surface]?.enabled === false ? "This bar is off: switch it on under Layout to set how it opens." : "Hot corners are for bars that open from their notches; this one is always open."),
      }),
      sec("behaviour", "Opening and closing"),
      part("general", "Behaviour", ["Hide in fullscreen"], { title: "Fullscreen apps" }),
    ],
  },
  {
    id: "size",
    label: "Size",
    icon: 0xe740,
    sections: [
      sec("widgets", "Layout for", { title: "" }),
      part("widgets", "Bar settings", ["Size at rest", "Size when open"], {
        title: "This bar's size",
        scope: "global",
        perBar: true,
        note: (cfg) => (cfg.surfaces?.[scope.surface]?.enabled === false ? "This bar is off: switch it on under Layout." : "Sizes at rest and when open are for docks: pick a dock above to see them."),
      }),
      part("appearance", "Bar and panels", ["Bar height", "Side padding", "Gap between widgets"], { title: "Bar" }),
      sec("appearance", "Bar sizes", { title: "Dock sizes" }),
    ],
  },
  { ...source("notches") },
  // Widgets
  { id: "widget-settings", label: "All widgets", icon: 0xeca5, sections: [{ title: "", fields: [{ kind: "custom", editor: "widgetCards" }] }] },
  ...WIDGETS.map(widgetPage),
  // Look
  {
    id: "colors",
    label: "Colours",
    icon: 0xe790,
    sections: [part("appearance", "Colors", ["Background tint", "Background", "Tint", "Opacity", "Text", "Secondary text opacity", "Use Windows' accent colour", "Accent", "Accent on widget details", "Hover highlights", "Warning"], { title: "Colours" })],
  },
  {
    id: "shape",
    label: "Shape",
    icon: 0xf156,
    sections: [
      sec("appearance", "Notch shape"),
      part("appearance", "Bar and panels", ["Panel fillet", "Panel corners"], { title: "Panels" }),
      sec("general", "Screen corners"),
    ],
  },
  {
    id: "text",
    label: "Text",
    icon: 0xe8d2,
    sections: [part("appearance", "Colors", ["<fontPicker>", "<fontPicker::secondary>", "Text size"], { title: "Fonts", help: "For every widget; one widget's own font is on its page (Widgets)." })],
  },
  { ...source("animation"), id: "motion", label: "Motion", sections: all("animation", (x) => x.group !== "Per widget") },
  // Shell
  {
    id: "start",
    label: "Start and taskbar",
    icon: 0xe782,
    sections: [
      { title: "Start menu", help: "The Windows key, the hotkey and fullscreen apps are with the launcher's other settings.", fields: [], link: ["Launcher settings", "w-launcher"] },
      part("general", "Behaviour", ["Hide the Windows taskbar", "<taskbarGuide>"], { title: "Taskbar" }),
    ],
  },
  { ...source("switchers") },
  {
    id: "keyboard",
    label: "Keyboard",
    icon: 0xe765,
    sections: [
      {
        title: "Start",
        help: "The Windows key and the launcher's own hotkey.",
        fields: part("general", "Launcher", ["Replace the Start menu", "Hotkey"], { title: "" }).fields,
      },
      {
        title: "Switching",
        help: "Alt + Tab between windows, Win + Space between keyboard languages, Win + 1…9 to the dock's apps.",
        fields: [
          ...part("switchers", "Alt + Tab", ["Kasumi's Alt + Tab"], { title: "" }).fields,
          ...part("switchers", "Languages (Win + Space)", ["Kasumi's language picker"], { title: "" }).fields,
          ...part("widget-settings", "Tasks", ["Win + 1…9 opens apps"], { title: "" }).fields,
        ],
      },
      { title: "Volume", fields: part("osd", "Volume", ["Take over the volume keys"], { title: "" }).fields },
    ],
  },
  {
    id: "osd",
    label: "Pop-ups",
    icon: 0xe7f4,
    sections: [
      ...all("osd", (x) => x.title !== "Notifications"),
      part("general", "Behaviour", ["Tooltips", "Other apps' tooltips too (experimental)"], { title: "Hints" }),
    ],
  },
  { id: "notifications", label: "Notifications", icon: 0xea8f, sections: [sec("osd", "Notifications")] },
  { ...source("context-menu"), label: "Context menus" },
  // Kasumi
  { ...source("presets") },
  { ...source("about"), label: "Updates and files" },
];

// --- Expert settings and tags --------------------------------------------------------

/** Settings most people never need, folded under their card's More, by page. (A section
 *  called Springs folds all but Bounce.) */
const EXPERT: Record<string, string[]> = {
  opening: ["Hot corner size", "Edge zone size", "Open delay", "Close delay", "Top edge band"],
  shape: ["Flare width", "Round-off width", "Panel fillet"],
  colors: ["Secondary text opacity"],
  osd: ["Pill width", "Pill height", "Length", "Width"],
};

/** What a setting changes or needs, by its name. */
const TAGS: Record<string, string> = {
  "Hide the Windows taskbar": "Changes Windows",
  "Replace the Start menu": "Changes Windows",
  "Take over the volume keys": "Changes Windows",
  Background: "Blur: Windows 11",
};

for (const p of pages) {
  for (const sc of p.sections) {
    for (const f of sc.fields) {
      if (!("label" in f)) continue;
      if ((sc.title === "Springs" && f.label !== "Bounce") || EXPERT[p.id]?.includes(f.label)) f.more = true;
      if (TAGS[f.label]) f.tag = TAGS[f.label];
    }
  }
}

/** The sidebar's places: each opens its first page; the others show under it while
 *  it's open. `icon` 0: Kasumi's logo. */
export interface Area {
  id: string;
  label: string;
  icon: number;
  pages: string[];
}

export const areas: Area[] = [
  { id: "home", label: "Home", icon: 0xe80f, pages: ["home"] },
  { id: "bars", label: "Bars and dock", icon: 0xe71d, pages: ["layout", "opening", "size", "notches"] },
  { id: "widgets", label: "Widgets", icon: 0xeca5, pages: ["widget-settings", ...WIDGETS.map((w) => `w-${w.id}`)] },
  { id: "look", label: "Look", icon: 0xe790, pages: ["colors", "shape", "text", "motion"] },
  { id: "windows", label: "Shell", icon: 0xe7c4, pages: ["start", "keyboard", "switchers", "osd", "notifications", "context-menu"] },
  { id: "kasumi", label: "Kasumi", icon: 0, pages: ["presets", "about"] },
];

/** Pages as they were once called (the bar menu, older links), and where they are now. */
export const pageAliases: Record<string, string> = {
  general: "home",
  widgets: "layout",
  appearance: "colors",
  animation: "motion",
  behaviour: "opening",
};

/** The place a page is in. */
export const areaOf = (page: string): Area => areas.find((a) => a.pages.includes(page)) ?? areas[0];

/** File Explorer styling (explorer.rs, explorer-tap/): not offered yet (the header's band
 *  isn't right); put it back in `pages` to show it. */
export const explorerPage: Page = {
    id: "explorer",
    label: "File Explorer",
    icon: 0xe8b7,
    sections: [
      {
        title: "File Explorer (experimental)",
        help: "Kasumi tidies File Explorer's folder windows: hides the parts of the top you don't use and styles the window. Hiding works from inside Explorer (a small library of Kasumi's, loaded into each folder window through Windows' XAML diagnostics); if a Windows update renames a part, it just stops being hidden.",
        fields: [{ kind: "toggle", label: "Style File Explorer", path: ["explorer", "enabled"], default: false }],
      },
      {
        title: "Hide",
        help: "Ticked parts disappear from every folder window, open ones too (within a second); unticked, they come back.",
        fields: [{ kind: "custom", editor: "explorerParts" }],
      },
      {
        title: "Window",
        help: "The frame around folder windows. A backdrop shows where Explorer lets it through (the title, tabs and toolbar).",
        fields: [
          {
            kind: "select",
            label: "Backdrop",
            path: (cfg) => (cfg.explorer?.enabled ? ["explorer", "backdrop"] : null),
            default: "default",
            options: [
              { value: "default", label: "Windows (as Windows has it)" },
              { value: "mica", label: "Mica" },
              { value: "mica_alt", label: "Mica Alt" },
              { value: "acrylic", label: "Acrylic" },
              { value: "none", label: "None" },
            ],
          },
          {
            kind: "select",
            label: "Title bar",
            path: (cfg) => (cfg.explorer?.enabled ? ["explorer", "title_bar"] : null),
            default: "default",
            options: [
              { value: "default", label: "Windows (as Windows has it)" },
              { value: "dark", label: "Dark" },
              { value: "light", label: "Light" },
            ],
          },
          {
            kind: "select",
            label: "Corners",
            path: (cfg) => (cfg.explorer?.enabled ? ["explorer", "corners"] : null),
            default: "default",
            options: [
              { value: "default", label: "Windows (as Windows has it)" },
              { value: "round", label: "Round" },
              { value: "small", label: "Slight (slightly round)" },
              { value: "square", label: "Square" },
            ],
          },
          {
            kind: "select",
            label: "Border",
            path: (cfg) => (cfg.explorer?.enabled ? ["explorer", "border"] : null),
            default: "default",
            options: [
              { value: "default", label: "Windows (as Windows has it)" },
              { value: "accent", label: "Accent (your accent colour)" },
              { value: "color", label: "Own (a colour of your own)" },
              { value: "none", label: "None" },
            ],
          },
          {
            kind: "color",
            label: "Border colour",
            path: (cfg) => (cfg.explorer?.enabled && cfg.explorer?.border === "color" ? ["explorer", "border_color"] : null),
            default: "#ffffff",
          },
          {
            kind: "toggle",
            label: "Explorer's dialogs too",
            help: "Properties, copying, confirmations get the same frame.",
            path: (cfg) => (cfg.explorer?.enabled ? ["explorer", "dialogs"] : null),
            default: true,
          },
        ],
      },
      {
        title: "Folder options",
        help: "Windows' own Explorer settings, in one place (as in Explorer's Folder Options); they work without Kasumi's styling too. The first ones change open windows at once, the rest the windows opened after.",
        fields: [{ kind: "custom", editor: "explorerOptions" }],
      },
      {
        title: "Advanced",
        collapsible: true,
        help: "For parts not in the list above.",
        fields: [
          {
            kind: "list",
            label: "More parts to hide",
            help: "Names from Explorer's map (Type#Name, a name, or Parent > Part), comma separated.",
            path: (cfg) => (cfg.explorer?.enabled ? ["explorer", "hide_custom"] : null),
            default: [],
            placeholder: "AppBarButton#upButton",
          },
          {
            kind: "toggle",
            label: "Map Explorer's parts",
            help: "Writes each folder window's parts to %TEMP%\\kasumi-explorer-<process>.txt, to find their names.",
            path: (cfg) => (cfg.explorer?.enabled ? ["explorer", "map"] : null),
            default: false,
          },
        ],
      },
    ],
  };
