// The settings window is generated from this declaration: every setting is
// listed once, with where it lives in config.toml, its control and default.
// Adding a feature's settings = adding entries here.

import { scope } from "./editors";

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

/** A setting of the surface selected on the Widgets page; `dockOnly` hides it for bars. */
const surfacePath =
  (key: string, dockOnly = false): PathSpec =>
  (cfg) => {
    const s = cfg.surfaces?.[scope.surface];
    if (!s || (dockOnly && s.mode !== "dock")) return null;
    return ["surfaces", scope.surface, key];
  };

/** Only for widgets that have their own notch (in the global layout). */
const hideWhenMaximized =
  (widget: string): PathSpec =>
  (cfg) =>
    [cfg.layout, ...Object.values<any>(cfg.surfaces ?? {})].some((l) => (l?.notch ?? []).includes(widget))
      ? ["widgets", widget, "hide_when_maximized"]
      : null;

export interface Section {
  title: string;
  help?: string;
  fields: Field[];
  /** Shown as a collapsible row (closed by default) with this icon. */
  collapsible?: boolean;
  icon?: number;
  /** Widgets page: only in the Global view, or only when a monitor is selected. */
  scope?: "global" | "monitor";
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
        ],
      },
      {
        title: "Hot corners",
        help: "Besides hovering a notch, the bar also opens when the cursor rests in any of these.",
        fields: [
          { kind: "toggle", label: "Top-left corner", path: ["timing", "hot_top_left"], default: true },
          { kind: "toggle", label: "Top-right corner", path: ["timing", "hot_top_right"], default: false },
          {
            kind: "toggle",
            label: "Anywhere along the top",
            help: "Pushing the cursor against the top of the screen opens the bar, wherever it is.",
            path: ["timing", "hot_top_edge"],
            default: false,
          },
          { kind: "slider", label: "Corner size", path: ["timing", "hot_corner"], default: 12, min: 4, max: 40, step: 1, unit: "px" },
        ],
      },
    ],
  },
  {
    id: "widgets",
    label: "Widgets",
    icon: 0xe71d,
    sections: [
      { title: "Layout for", fields: [{ kind: "custom", editor: "scopeSelector" }] },
      {
        title: "Bar & dock",
        scope: "global",
        help: "A bar opens into a full-width bar; a dock's notches grow in place.",
        fields: [
          { kind: "custom", editor: "surfaceManager" },
          {
            kind: "select",
            label: "Style",
            path: surfacePath("mode"),
            default: "bar",
            options: [
              { value: "bar", label: "Status bar" },
              { value: "dock", label: "Dock" },
            ],
          },
          {
            kind: "select",
            label: "Edge",
            path: surfacePath("edge"),
            default: "top",
            options: [
              { value: "top", label: "Top" },
              { value: "bottom", label: "Bottom" },
            ],
          },
          {
            kind: "select",
            label: "Size at rest",
            help: "Tiny is a thin handle that opens on hover.",
            path: surfacePath("rest", true),
            default: "small",
            options: [
              { value: "tiny", label: "Tiny" },
              { value: "small", label: "Small" },
              { value: "medium", label: "Medium" },
            ],
          },
          {
            kind: "select",
            label: "Size when open",
            path: surfacePath("open", true),
            default: "small",
            options: [
              { value: "small", label: "Small" },
              { value: "medium", label: "Medium" },
            ],
          },
          {
            kind: "toggle",
            label: "Shrink when a window covers it",
            help: "Shrinks to the tiny handle while a maximized window is under it.",
            path: surfacePath("shrink_when_covered", true),
            default: true,
          },
        ],
      },
      { title: "This monitor", scope: "monitor", fields: [{ kind: "custom", editor: "monitorToggles" }] },
      { title: "Placement", fields: [{ kind: "custom", editor: "placement" }] },
      { title: "Own notches on this monitor", scope: "monitor", fields: [{ kind: "custom", editor: "monitorNotches" }] },
      {
        title: "Notches",
        scope: "global",
        help: "Widgets with “Own notch” on show as a notch on their side while the bar is closed.",
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
          { kind: "toggle", label: "Left: sit in the corner", path: ["notch", "left", "hug"], default: true },
          {
            kind: "toggle",
            label: "Left: move aside for windows",
            help: "Slide to the offset while a maximized window's caption buttons are there (back in the corner while the bar is open).",
            path: ["notch", "left", "avoid_windows"],
            default: true,
          },
          { kind: "number", label: "Left: offset", help: "From the left edge, when moved aside or not in the corner.", path: ["notch", "left", "offset"], default: 150, min: 0, max: 2000, step: 1, unit: "px" },
          { kind: "toggle", label: "Right: sit in the corner", path: ["notch", "right", "hug"], default: true },
          {
            kind: "toggle",
            label: "Right: move aside for windows",
            help: "Slide to the offset while a maximized window's caption buttons are there (back in the corner while the bar is open).",
            path: ["notch", "right", "avoid_windows"],
            default: true,
          },
          { kind: "number", label: "Right: offset", help: "From the right edge, when moved aside or not in the corner.", path: ["notch", "right", "offset"], default: 150, min: 0, max: 2000, step: 1, unit: "px" },
          { kind: "number", label: "Center: offset", help: "Shift from the middle.", path: ["notch", "center", "offset"], default: 0, min: -2000, max: 2000, step: 1, unit: "px" },
        ],
      },
      {
        title: "Clock",
        scope: "global",
        collapsible: true,
        icon: 0xe823,
        fields: [{ kind: "custom", editor: "notch:clock" }, { kind: "toggle", label: "Hide under maximized windows", help: "Instead of moving aside with its notch, this widget hides while a window covers the notch's spot. Back when the bar opens.", path: hideWhenMaximized("clock"), default: false }, { kind: "custom", editor: "clockFormat" }],
      },
      {
        title: "PC temps",
        scope: "global",
        collapsible: true,
        icon: 0xe9ca,
        fields: [
          { kind: "custom", editor: "notch:temps" }, { kind: "toggle", label: "Hide under maximized windows", help: "Instead of moving aside with its notch, this widget hides while a window covers the notch's spot. Back when the bar opens.", path: hideWhenMaximized("temps"), default: false },
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
        scope: "global",
        collapsible: true,
        icon: 0xe706,
        fields: [
          { kind: "custom", editor: "notch:weather" }, { kind: "toggle", label: "Hide under maximized windows", help: "Instead of moving aside with its notch, this widget hides while a window covers the notch's spot. Back when the bar opens.", path: hideWhenMaximized("weather"), default: false },
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
          { kind: "slider", label: "Hours in the panel", path: ["widgets", "weather", "hours"], default: 8, min: 3, max: 12, step: 1 },
          { kind: "slider", label: "Refresh every", path: ["widgets", "weather", "interval_min"], default: 15, min: 5, max: 60, step: 5, unit: "min" },
        ],
      },
      {
        title: "Media",
        scope: "global",
        collapsible: true,
        icon: 0xe8d6,
        fields: [
          { kind: "custom", editor: "notch:media" }, { kind: "toggle", label: "Hide under maximized windows", help: "Instead of moving aside with its notch, this widget hides while a window covers the notch's spot. Back when the bar opens.", path: hideWhenMaximized("media"), default: false },
          { kind: "slider", label: "Title width", path: ["widgets", "media", "max_width"], default: 240, min: 100, max: 500, step: 10, unit: "px" },
          {
            kind: "toggle",
            label: "Scroll long titles",
            help: "Titles wider than the title width scroll instead of ending in “…”.",
            path: ["widgets", "media", "scroll"],
            default: false,
          },
        ],
      },
      {
        title: "Tray",
        scope: "global",
        collapsible: true,
        icon: 0xe70d,
        fields: [
          { kind: "custom", editor: "notch:tray" }, { kind: "toggle", label: "Hide under maximized windows", help: "Instead of moving aside with its notch, this widget hides while a window covers the notch's spot. Back when the bar opens.", path: hideWhenMaximized("tray"), default: false },
          {
            kind: "toggle",
            label: "Host tray icons",
            help: "Collect apps' tray icons (Explorer's tray keeps working). Applies on the next launch.",
            path: ["widgets", "tray", "host"],
            default: true,
          },
          { kind: "slider", label: "Icons per row", path: ["widgets", "tray", "icons_per_row"], default: 5, min: 3, max: 10, step: 1 },
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
        scope: "global",
        collapsible: true,
        icon: 0xe765,
        fields: [{ kind: "custom", editor: "notch:language" }, { kind: "toggle", label: "Hide under maximized windows", help: "Instead of moving aside with its notch, this widget hides while a window covers the notch's spot. Back when the bar opens.", path: hideWhenMaximized("language"), default: false }],
      },
      {
        title: "Menu",
        scope: "global",
        collapsible: true,
        icon: 0xe700,
        fields: [{ kind: "custom", editor: "notch:menu" }, { kind: "toggle", label: "Hide under maximized windows", help: "Instead of moving aside with its notch, this widget hides while a window covers the notch's spot. Back when the bar opens.", path: hideWhenMaximized("menu"), default: false }, { kind: "custom", editor: "menuItems" }],
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
          { kind: "color", label: "Background", path: ["theme", "background"], default: "#000000" },
          { kind: "color", label: "Text", path: ["theme", "text"], default: "#ffffff" },
          { kind: "slider", label: "Secondary text opacity", path: ["theme", "dim_opacity"], default: 0.45, min: 0.2, max: 0.9, step: 0.05 },
          { kind: "color", label: "Accent", help: "Today in the calendar, sliders and bars.", path: ["theme", "accent"], default: "#ffffff" },
          { kind: "color", label: "Warning", help: "Hot temperatures.", path: ["theme", "warn"], default: "#ff9f43" },
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
        ],
      },
      {
        title: "Springs",
        help: "Higher stiffness is faster; lower damping is bouncier.",
        fields: [
          { kind: "slider", label: "Bar stiffness", path: ["animation", "expand_stiffness"], default: 3440, min: 200, max: 6000, step: 20 },
          { kind: "slider", label: "Bar damping", path: ["animation", "expand_damping"], default: 92, min: 10, max: 160, step: 1 },
          { kind: "slider", label: "Spread / slide stiffness", path: ["animation", "spread_stiffness"], default: 2120, min: 200, max: 6000, step: 20 },
          { kind: "slider", label: "Spread / slide damping", path: ["animation", "spread_damping"], default: 80, min: 10, max: 160, step: 1 },
          { kind: "slider", label: "Panel stiffness", path: ["animation", "panel_stiffness"], default: 775, min: 200, max: 6000, step: 20 },
          { kind: "slider", label: "Panel damping", path: ["animation", "panel_damping"], default: 43, min: 10, max: 160, step: 1 },
        ],
      },
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
        fields: [
          { kind: "toggle", label: "Show OSD", path: ["osd", "enabled"], default: true },
          { kind: "slider", label: "Duration", path: ["osd", "duration_ms"], default: 1500, min: 500, max: 4000, step: 100, unit: "ms" },
          { kind: "slider", label: "Pill width", path: ["osd", "width"], default: 230, min: 160, max: 400, step: 5, unit: "px" },
          { kind: "slider", label: "Pill height", path: ["osd", "height"], default: 34, min: 24, max: 48, step: 1, unit: "px" },
        ],
      },
      {
        title: "Volume",
        fields: [
          {
            kind: "toggle",
            label: "Take over the volume keys",
            help: "Hides the Windows volume flyout; StatusNotch sets the volume.",
            path: ["osd", "replace_volume_keys"],
            default: true,
          },
          { kind: "slider", label: "Volume step", path: ["osd", "volume_step"], default: 2, min: 1, max: 10, step: 1, unit: "%" },
        ],
      },
      {
        title: "Other pops",
        fields: [
          { kind: "toggle", label: "Brightness (Fn keys)", path: ["osd", "brightness"], default: true },
          { kind: "toggle", label: "Keyboard language", path: ["osd", "keyboard"], default: true },
          { kind: "slider", label: "Language pop width", path: ["osd", "keyboard_width"], default: 120, min: 80, max: 200, step: 5, unit: "px" },
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
