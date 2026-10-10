// Structured editors for the settings window: things richer than a single
// control (widget placement, notches, menu entries, monitors, lookups).
import { invoke } from "@tauri-apps/api/core";
import { POWER_ACTIONS } from "../power-actions";
import { DEFAULT_TILES, MAX_TILES, QUICK_TILES } from "../quick-tiles";
import { $, glyph, iconButton, input, row, select, toggleSwitch } from "./dom";
import type { Cfg, Path } from "./schema";

export interface Op {
  path: Path;
  value: unknown;
}

export interface EditorCtx {
  cfg: Cfg;
  /** Writes one value (null removes). */
  save(path: Path, value: unknown): Promise<void>;
  /** Writes several edits as one validated change. */
  saveMany(ops: Op[]): Promise<void>;
  toast(text: string, error?: boolean): void;
  /** Redraws the page (e.g. after switching Global / monitor). */
  rerender(): void;
  /** Re-reads config.toml (e.g. after applying a preset) and redraws. */
  reload(): Promise<void>;
  /** Shows another page with one of its dropdowns open (by its section title). */
  goto(page: string, section: string): void;
  /** The part after ":" in the field's editor name, if any. */
  arg?: string;
}

type Editor = (ctx: EditorCtx) => HTMLElement;

const sw = (on: boolean, onChange: (on: boolean) => void) => toggleSwitch(on, onChange).el;

// --- Widgets & layout model ------------------------------------------------------
//
// Config model: each surface ([surfaces.top], [surfaces.bottom], ...) has left /
// center / right lists (a side for every shown widget) and `notch` (which of them
// show as their own notch). [[monitors]] rules may carry their own lists per
// surface (`surfaces = { top = { ... } }`); `custom_layout` / `follow_unchanged`
// there are UI flags. Older configs had a single [layout] (the top bar).

export const WIDGETS: { id: string; label: string; icon: number }[] = [
  { id: "clock", label: "Clock", icon: 0xe823 },
  { id: "temps", label: "PC temps", icon: 0xe9ca },
  { id: "weather", label: "Weather", icon: 0xe706 },
  { id: "media", label: "Media", icon: 0xe8d6 },
  { id: "tray", label: "Tray", icon: 0xe70d },
  { id: "language", label: "Language", icon: 0xe765 },
  { id: "menu", label: "Menu", icon: 0xe700 },
  { id: "tasks", label: "Tasks", icon: 0xe737 },
  { id: "quick", label: "Quick settings", icon: 0xe713 },
  { id: "buttons", label: "Buttons", icon: 0xe7e8 },
  { id: "launcher", label: "Launcher", icon: 0xe721 },
  { id: "focus", label: "Focused app", icon: 0xe7c4 },
  { id: "netspeed", label: "Network speed", icon: 0xe74b },
  { id: "disk", label: "Disk usage", icon: 0xeda2 },
  { id: "privacy", label: "Privacy", icon: 0xe720 },
  { id: "claude", label: "Claude Code", icon: 0xe8bd },
  { id: "clipboard", label: "Clipboard", icon: 0xe77f },
  { id: "notifications", label: "Notifications", icon: 0xea8f },
];

/** The Widgets page section holding a widget's settings. */
const SECTION: Record<string, string> = {
  clock: "Clock",
  temps: "PC temps",
  weather: "Weather",
  media: "Media",
  tray: "Tray",
  language: "Keyboard language",
  menu: "Menu",
  tasks: "Tasks",
  quick: "Quick settings",
  buttons: "Buttons",
  launcher: "Launcher",
  focus: "Focused app",
  netspeed: "Network speed",
  disk: "Disk usage",
  privacy: "Privacy",
  claude: "Claude Code",
  clipboard: "Clipboard",
  notifications: "Notifications",
};
const widgetSection = (id: string) => SECTION[id.split("#")[0]] ?? "";

/** A placed widget's type: instances are named `<type>#<n>`. */
const typeOf = (id: string) => id.split("#")[0];

/** The built-in widgets plus their extra instances in the config (`buttons#2`, ...). */
export function catalog(cfg: Cfg): { id: string; label: string; icon: number }[] {
  const out = [...WIDGETS];
  const names = new Set<string>(Object.keys(cfg.widgets ?? {}));
  for (const s of Object.values<any>(cfg.surfaces ?? {})) for (const k of ["left", "center", "right"]) for (const n of s?.[k] ?? []) names.add(n);
  for (const n of [...names].sort()) {
    const m = /^(\w+)#(\d+)$/.exec(n);
    const base = m && WIDGETS.find((w) => w.id === m[1]);
    if (base && !out.some((w) => w.id === n)) out.push({ id: n, label: `${base.label} ${m![2]}`, icon: base.icon });
  }
  return out;
}

export interface LayoutCfg {
  left: string[];
  center: string[];
  right: string[];
  notch: string[];
}
type Side = "left" | "center" | "right";
const SIDES: Side[] = ["left", "center", "right"];

const enabled = (cfg: Cfg, id: string) => cfg.widgets?.[id]?.enabled !== false;
const emptyLayout = (): LayoutCfg => ({ left: [], center: [], right: [], notch: [] });
/** Just the four lists of a layout-like table (a surface also has edge, mode, ...). */
const normalize = (l: Partial<LayoutCfg> | undefined): LayoutCfg => ({
  left: [...(l?.left ?? [])],
  center: [...(l?.center ?? [])],
  right: [...(l?.right ?? [])],
  notch: [...(l?.notch ?? [])],
});
const LISTS = ["left", "center", "right", "notch"] as const;
const hasLists = (o: any) => !!o && LISTS.some((k) => Array.isArray(o[k]));
const sameLayout = (a: LayoutCfg, b: LayoutCfg) =>
  SIDES.every((s) => JSON.stringify(a[s]) === JSON.stringify(b[s])) &&
  JSON.stringify([...a.notch].sort()) === JSON.stringify([...b.notch].sort());
const sideOf = (l: LayoutCfg, id: string): Side | null => SIDES.find((s) => l[s].includes(id)) ?? null;

/** The top and bottom bars (always, on or off), then any other surfaces in the config. */
export const surfaceIds = (cfg: Cfg): string[] => [
  "top",
  "bottom",
  ...Object.keys(cfg.surfaces ?? {})
    .filter((id) => id !== "top" && id !== "bottom")
    .sort(),
];

/** Surfaces that exist and are switched on. */
const shownSurfaces = (cfg: Cfg): string[] =>
  surfaceIds(cfg).filter((id) => (id === "top" && !cfg.surfaces) || (cfg.surfaces?.[id] && cfg.surfaces[id].enabled !== false));

/** A surface's global layout. */
export const surfaceLayout = (cfg: Cfg, id: string): LayoutCfg =>
  cfg.surfaces ? normalize(cfg.surfaces[id]) : id === "top" ? normalize(cfg.layout) : emptyLayout();

/** Which part of the Widgets page is being edited: a surface, globally or for one monitor. */
export const scope: { monitor: string | null; surface: string } = { monitor: null, surface: "top" };

let monitorsCache: MonitorInfo[] | null = null;
/** The monitors (asked for once). */
const loadMonitors = (): Promise<MonitorInfo[]> =>
  monitorsCache ? Promise.resolve(monitorsCache) : invoke<MonitorInfo[]>("monitors_list").then((list) => (monitorsCache = list));

interface MonitorInfo {
  index: number;
  name: string;
  primary: boolean;
  width: number;
  height: number;
}

/** The rule written for a monitor by name (ours), if any. */
function ownRule(cfg: Cfg, name: string): [number, any] | null {
  const rules: any[] = cfg.monitors ?? [];
  const i = rules.findIndex((r) => String(r.match ?? "").toLowerCase() === name.toLowerCase());
  return i < 0 ? null : [i, rules[i]];
}

/** Mirrors the engine's rule matching (first match wins). */
function ruleFor(cfg: Cfg, m: MonitorInfo): any | undefined {
  return (cfg.monitors ?? []).find((r: any) => {
    const s = String(r.match ?? "*").trim();
    const l = s.toLowerCase();
    if (l === "*") return true;
    if (l === "primary") return m.primary;
    if (s.startsWith("#")) return Number(s.slice(1)) === m.index;
    return m.name.toLowerCase().includes(l);
  });
}

/** The layout a monitor actually shows on a surface (default: the selected one). */
function monitorLayout(cfg: Cfg, name: string, surface = scope.surface): LayoutCfg {
  const m = monitorsCache?.find((x) => x.name === name);
  const r = m ? ruleFor(cfg, m) : ownRule(cfg, name)?.[1];
  const own = r?.surfaces?.[surface];
  if (hasLists(own)) return normalize(own);
  return surfaceLayout(cfg, surface);
}

/** Whether a monitor has its own layout of a bar (else it follows the global one). */
export function ownsLayout(cfg: Cfg, name: string, surface = scope.surface): boolean {
  const m = monitorsCache?.find((x) => x.name === name);
  const r = m ? ruleFor(cfg, m) : ownRule(cfg, name)?.[1];
  return hasLists(r?.surfaces?.[surface]);
}

/** A surface's layout in the current scope (global, or the selected monitor's). */
const scopedLayout = (cfg: Cfg, surface: string): LayoutCfg =>
  scope.monitor ? monitorLayout(cfg, scope.monitor, surface) : surfaceLayout(cfg, surface);

/** A copy of the config with `id` taken out of every layout (all surfaces, all monitors). */
function stripped(cfg: Cfg, id: string): Cfg {
  const out = structuredClone(cfg);
  const clean = (o: any) => {
    for (const k of LISTS) if (Array.isArray(o?.[k])) o[k] = o[k].filter((x: string) => x !== id);
  };
  for (const sc of Object.values<any>(out.surfaces ?? {})) clean(sc);
  for (const r of out.monitors ?? []) {
    for (const o of Object.values<any>(r.surfaces ?? {})) clean(o);
    clean(r.layout);
  }
  return out;
}

/** Edits turning `cfg`'s layouts into `next`'s (surface lists, then monitor rules). */
function layoutDiffOps(cfg: Cfg, next: Cfg): Op[] {
  const ops: Op[] = [];
  for (const [s, sc] of Object.entries<any>(next.surfaces ?? {})) {
    for (const k of LISTS) {
      if (JSON.stringify(sc?.[k]) !== JSON.stringify(cfg.surfaces?.[s]?.[k])) ops.push({ path: ["surfaces", s, k], value: sc[k] ?? [] });
    }
  }
  if (JSON.stringify(next.monitors) !== JSON.stringify(cfg.monitors)) ops.push({ path: ["monitors"], value: next.monitors });
  return ops;
}

/**
 * Switched-off widgets have no spot anywhere (older versions kept it, so turning
 * one back on resurfaced it in every layout that still listed it).
 */
export function migrationOpsDisabledSpots(cfg: Cfg): Op[] {
  let next = cfg;
  for (const [id, w] of Object.entries<any>(cfg.widgets ?? {})) if (w?.enabled === false) next = stripped(next, id);
  return next === cfg ? [] : layoutDiffOps(cfg, next);
}

/** Widgets that no longer exist: out of every layout, their settings gone. */
const REMOVED_WIDGETS = ["recycle"];
export function migrationOpsRemovedWidgets(cfg: Cfg): Op[] {
  let next = cfg;
  const ops: Op[] = [];
  for (const id of REMOVED_WIDGETS) {
    next = stripped(next, id);
    if (cfg.widgets?.[id] !== undefined) ops.push({ path: ["widgets", id], value: null });
  }
  return [...layoutDiffOps(cfg, next), ...ops];
}

/** Rewrites (or creates, before the catch-all) the named monitor's rule. */
function ruleOps(cfg: Cfg, name: string, update: (r: any) => void): Op[] {
  const rules: any[] = structuredClone(cfg.monitors ?? []);
  let i = rules.findIndex((r) => String(r.match ?? "").toLowerCase() === name.toLowerCase());
  if (i < 0) {
    const m = monitorsCache?.find((x) => x.name === name);
    // Start from whatever currently applies to this monitor, so nothing changes by itself.
    const current = m ? ruleFor(cfg, m) : undefined;
    const fresh: any = { match: name, enabled: current?.enabled !== false };
    if (current?.surfaces) fresh.surfaces = structuredClone(current.surfaces);
    const star = rules.findIndex((r) => String(r.match ?? "*").trim() === "*");
    i = star < 0 ? rules.length : star;
    rules.splice(i, 0, fresh);
  }
  update(rules[i]);
  for (const r of rules) {
    delete r.notches; // legacy fields
    delete r.bar;
    delete r.layout;
  }
  return [{ path: ["monitors"], value: rules }];
}

/** Edits that store `next` as the selected surface's layout in the current scope. */
function layoutOps(cfg: Cfg, next: LayoutCfg): Op[] {
  return layoutsOps(cfg, { [scope.surface]: next });
}

/** Edits that store new layouts for several surfaces in the current scope. */
function layoutsOps(cfg: Cfg, changed: Record<string, LayoutCfg>): Op[] {
  if (!scope.monitor) {
    return Object.entries(changed).flatMap(([s, next]) => LISTS.map((k) => ({ path: ["surfaces", s, k], value: next[k] })));
  }
  return ruleOps(cfg, scope.monitor, (r) => {
    for (const [s, next] of Object.entries(changed)) {
      const own = ((r.surfaces ??= {})[s] ??= {});
      own.custom_layout = true;
      // "Follow global while unchanged": a layout equal to the global one isn't stored.
      const follow = own.follow_unchanged !== false && sameLayout(next, surfaceLayout(cfg, s));
      for (const k of LISTS) {
        if (follow) delete own[k];
        else own[k] = next[k];
      }
    }
  });
}

/** Global / per-monitor selector at the top of the Widgets page. */
const scopeSelector: Editor = (ctx) => {
  const bar = $("div", "scope-bar");
  const draw = (list: MonitorInfo[]) => {
    bar.replaceChildren();
    const chip = (label: string, value: string | null, icon: number) => {
      const b = $("button", `scope-chip${scope.monitor === value ? " active" : ""}`, glyph(icon), label);
      b.addEventListener("click", () => {
        scope.monitor = value;
        ctx.rerender();
      });
      bar.append(b);
    };
    chip("Global", null, 0xe774);
    for (const m of list) {
      chip(m.primary ? `${m.name} · primary` : m.name, m.name, 0xe7f4);
      // Monitors with their own layout of the selected bar are marked.
      if (ownsLayout(ctx.cfg, m.name)) {
        const b = bar.lastElementChild as HTMLElement;
        b.append($("span", "scope-own", "own layout"));
        b.title = `${m.name} has its own layout of this bar`;
      }
    }
  };
  loadMonitors().then(draw);
  const help = scope.monitor
    ? `Layout for ${scope.monitor}. Widget settings below are shared by all monitors.`
    : "The layout every monitor uses unless it has its own.";
  const ids = surfaceIds(ctx.cfg);
  if (!ids.includes(scope.surface)) scope.surface = ids.includes("top") ? "top" : ids[0];
  if (ids.length < 2) return $("div", "", bar, $("p", "card-help", help));
  // Several bars/docks: pick which one the placement below edits.
  const surfaces = $("div", "scope-bar");
  for (const id of ids) {
    const s = ctx.cfg.surfaces?.[id] ?? {};
    const b = $("button", `scope-chip${scope.surface === id ? " active" : ""}${s.enabled === false || (!ctx.cfg.surfaces?.[id] && id !== "top") ? " off" : ""}`, glyph(surfaceIcon(id)), surfaceName(id, s));
    b.addEventListener("click", () => {
      scope.surface = id;
      ctx.rerender();
    });
    surfaces.append(b);
  }
  return $("div", "stack-tight", surfaces, bar, $("p", "card-help", help));
};

/** "Show the top / bottom bar": switching the bottom bar on the first time makes it a dock. */
const surfaceManager: Editor = (ctx) => {
  const cur = scope.surface;
  const s = ctx.cfg.surfaces?.[cur];
  const on = !!s && s.enabled !== false;
  return row(
    `Show the ${surfaceName(cur, s).toLowerCase()}`,
    cur === "bottom" ? "Starts as a dock: its notches grow in place when hovered (Style below)." : undefined,
    sw(on, (v) => {
      if (v && !s) {
        const dock = cur === "bottom";
        ctx.saveMany([
          { path: ["surfaces", cur, "mode"], value: dock ? "dock" : "bar" },
          ...(dock
            ? [
                { path: ["surfaces", cur, "rest"], value: "small" },
                { path: ["surfaces", cur, "open"], value: "medium" },
              ]
            : []),
          ...LISTS.map((k) => ({ path: ["surfaces", cur, k], value: [] as string[] })),
        ]);
      } else {
        ctx.saveMany([{ path: ["surfaces", cur, "enabled"], value: v }]);
      }
    }),
  );
};

/** Style of the selected bar; picking "Always on" also reserves its space. */
const surfaceStyle: Editor = (ctx) => {
  const cur = scope.surface;
  const s = ctx.cfg.surfaces?.[cur];
  if (!s || s.enabled === false) return $("div");
  const sel = select(
    [
      ["bar", "Status bar"],
      ["dock", "Dock"],
      ["always", "Always on"],
    ],
    s.mode ?? "bar",
  );
  sel.addEventListener("change", () => {
    const ops: Op[] = [{ path: ["surfaces", cur, "mode"], value: sel.value }];
    if (sel.value === "always") ops.push({ path: ["surfaces", cur, "reserve_space"], value: true });
    ctx.saveMany(ops);
  });
  return row(
    "Style",
    "Status bar: notches that open into a full-width bar. Dock: notches that grow in place. Always on: the full bar, always shown (reserves its space).",
    sel,
  );
};

/**
 * v4 -> v5: hot corners belong to each bar; the old [timing] switches (top bar
 * only) move into [surfaces.top].
 */
export function migrationOpsV5(cfg: Cfg): Op[] {
  const t = cfg.timing ?? {};
  const keys = ["hot_top_left", "hot_top_right", "hot_top_edge", "hot_corner", "hot_corner_enabled", "hot_corner_position"];
  if (!cfg.surfaces || !keys.some((k) => t[k] !== undefined)) return [];
  const on = t.hot_corner_enabled !== false;
  const pos = t.hot_corner_position ?? "top_left";
  const top = cfg.surfaces.top ?? {};
  const ops: Op[] = [];
  const set = (key: string, value: unknown) => {
    if (top[key] === undefined) ops.push({ path: ["surfaces", "top", key], value });
  };
  set("hot_left", on && (t.hot_top_left ?? pos === "top_left"));
  set("hot_right", on && (t.hot_top_right ?? pos === "top_right"));
  set("hot_edge", on && (t.hot_top_edge ?? pos === "top_edge"));
  set("hot_corner_size", t.hot_corner ?? 12);
  for (const k of keys) if (t[k] !== undefined) ops.push({ path: ["timing", k], value: null });
  return ops;
}

/** "Top bar", "Bottom bar" (any other surface: its id). */
export function surfaceName(id: string, _s?: any): string {
  return id === "top" ? "Top bar" : id === "bottom" ? "Bottom bar" : id;
}
const surfaceIcon = (id: string) => (id === "bottom" ? 0xe70d : 0xe70e); // chevron down / up

/** Per-monitor: StatusNotch on/off, custom layout, follow global while unchanged. */
const monitorToggles: Editor = (ctx) => {
  const name = scope.monitor!;
  const r = ownRule(ctx.cfg, name)?.[1];
  const m = monitorsCache?.find((x) => x.name === name);
  const effective = (m && ruleFor(ctx.cfg, m)) ?? r;
  const set = (update: (r: any) => void) => ctx.saveMany(ruleOps(ctx.cfg, name, update));
  const own = effective?.surfaces?.[scope.surface];
  const custom = hasLists(own) || own?.custom_layout === true;
  const setOwn = (update: (o: any) => void) => set((x) => update(((x.surfaces ??= {})[scope.surface] ??= {})));
  const box = $("div", "stack-tight");
  box.append(
    row("StatusNotch on this monitor", undefined, sw(effective?.enabled !== false, (on) => set((x) => (x.enabled = on)))),
    ...(surfaceIds(ctx.cfg).length > 1
      ? [
          row(
            `Show the ${surfaceName(scope.surface, ctx.cfg.surfaces?.[scope.surface]).toLowerCase()} here`,
            undefined,
            sw(own?.enabled !== false, (on) => setOwn((o) => (on ? delete o.enabled : (o.enabled = false)))),
          ),
        ]
      : []),
    row(
      "Custom layout",
      "Off: this monitor follows the global layout.",
      sw(custom, (on) =>
        setOwn((o) => {
          o.custom_layout = on;
          if (!on) for (const k of LISTS) delete o[k];
        }),
      ),
    ),
  );
  if (custom) {
    box.append(
      row(
        "Follow global while unchanged",
        "Until you change this monitor's layout, global changes still apply to it.",
        sw(own?.follow_unchanged !== false, (on) => setOwn((o) => (o.follow_unchanged = on))),
      ),
    );
  }
  return box;
};

/**
 * Drag widgets between Left / Center / Right of the selected surface, the other
 * surfaces and Disabled. A widget lives in one place: putting it somewhere takes
 * it out of wherever it was.
 */
/** The placement chips' two buttons, also explained in the Placement legend. */
export const CHIP_LEGEND: [number, string][] = [
  [0xe718, "Own notch"],
  [0xed1a, "Hides under maximized windows"],
];

const placement: Editor = (ctx) => {
  const cur = scope.surface;
  const ids = surfaceIds(ctx.cfg);
  const layouts: Record<string, LayoutCfg> = Object.fromEntries(ids.map((s) => [s, scopedLayout(ctx.cfg, s)]));
  const L = layouts[cur];
  const cat = catalog(ctx.cfg);
  const known = (id: string) => cat.some((w) => w.id === id);
  const placedOn = (id: string) => ids.filter((s) => sideOf(layouts[s], id));
  const isOff = (id: string) => !enabled(ctx.cfg, id) || placedOn(id).length === 0;
  const root = $("div", "sorter");
  let dragging: string | null = null;

  type Bucket = Side | "disabled" | `surface:${string}`;
  const columns: [Bucket, string][] = [
    ["left", "Left"],
    ["center", "Center"],
    ["right", "Right"],
    ...shownSurfaces(ctx.cfg)
      .filter((s) => s !== cur)
      .map((s): [Bucket, string] => [`surface:${s}`, `In the ${surfaceName(s).toLowerCase()}`]),
    ["disabled", "Disabled"],
  ];
  const listOf = (bucket: Bucket): string[] => {
    if (bucket === "disabled") return cat.map((w) => w.id).filter(isOff);
    if (bucket.startsWith("surface:")) {
      const l = layouts[bucket.slice(8)];
      return SIDES.flatMap((side) => l[side]).filter((id) => enabled(ctx.cfg, id) && known(id));
    }
    return L[bucket as Side].filter((id) => enabled(ctx.cfg, id) && known(id));
  };

  /**
   * A chip's buttons: own notch (bars: a notch on its side while the bar is closed;
   * a dock's widgets are all notches, an always-on bar has none) and hide under
   * maximized windows (a [widgets] option, so the same on every bar; it only does
   * anything in a notch). On another bar's column they just show the state.
   */
  const chipButtons = (id: string, where: string, elsewhere: boolean) => {
    const mode = ctx.cfg.surfaces?.[where]?.mode ?? "bar";
    const notch = mode === "dock" || (mode === "bar" && layouts[where].notch.includes(id));
    const hides = ctx.cfg.widgets?.[id]?.hide_when_maximized === true;
    const button = (code: number, on: boolean, usable: boolean, title: string, toggle: () => void) => {
      const b = $("button", `chip-button${on ? " on" : ""}`, glyph(code)) as HTMLButtonElement;
      b.title = title;
      b.draggable = false;
      b.disabled = elsewhere || !usable;
      b.addEventListener("click", (ev) => {
        ev.stopPropagation();
        toggle();
      });
      return b;
    };
    const pin = button(
      CHIP_LEGEND[0][0],
      notch,
      mode === "bar",
      mode === "dock"
        ? "Own notch: every widget in a dock is a notch"
        : mode !== "bar"
          ? "Own notch: an always-shown bar has no notches"
          : notch
            ? "Own notch: shows as a notch on its side while the bar is closed. Click to stop"
            : "Own notch: click to also show it as a notch on its side while the bar is closed",
      () => {
        const next = structuredClone(L);
        next.notch = notch ? next.notch.filter((x) => x !== id) : [...next.notch, id];
        ctx.saveMany(layoutOps(ctx.cfg, next));
      },
    );
    const hide = button(
      CHIP_LEGEND[1][0],
      hides && notch,
      notch,
      !notch
        ? "Hides under maximized windows: only for widgets in a notch"
        : hides
          ? "Hides while a maximized window covers its notch (on every bar). Click to keep it"
          : "Click to hide it while a maximized window covers its notch, instead of moving aside (on every bar)",
      () => ctx.saveMany([{ path: ["widgets", id, "hide_when_maximized"], value: hides ? null : true }]),
    );
    return $("div", "chip-buttons", pin, hide);
  };

  for (const [bucket, title] of columns) {
    const elsewhere = bucket.startsWith("surface:");
    const list = $("div", "bucket-list");
    for (const id of listOf(bucket)) {
      const w = cat.find((x) => x.id === id)!;
      const chip = $("div", "chip", glyph(w.icon), w.label);
      const where = elsewhere ? bucket.slice(8) : cur;
      if (bucket !== "disabled") chip.append(chipButtons(id, where, elsewhere));
      chip.draggable = true;
      chip.dataset.id = id;
      // A click (not a drag) opens the widget's own settings.
      chip.title = `${w.label} settings`;
      chip.addEventListener("click", () => ctx.goto("widget-settings", widgetSection(id)));
      chip.addEventListener("dragstart", (e) => {
        dragging = id;
        e.dataTransfer?.setData("text/plain", id);
        chip.classList.add("dragging");
      });
      chip.addEventListener("dragend", () => {
        dragging = null;
        chip.classList.remove("dragging");
        root.querySelectorAll(".drop-before").forEach((x) => x.classList.remove("drop-before"));
      });
      list.append(chip);
    }
    const strip = bucket === "disabled" || elsewhere;
    const col = $("div", `bucket${strip ? " disabled" : ""}${elsewhere ? " elsewhere" : ""}`, $("div", "bucket-title", title), list);
    const indexAt = (y: number) => {
      const chips = [...list.querySelectorAll<HTMLElement>(".chip:not(.dragging)")];
      const i = chips.findIndex((c) => y < c.getBoundingClientRect().top + c.offsetHeight / 2);
      return { i: i < 0 ? chips.length : i, chips };
    };
    col.addEventListener("dragover", (e) => {
      if (!dragging) return;
      e.preventDefault();
      col.classList.add("over");
      const { i, chips } = indexAt(e.clientY);
      chips.forEach((c, j) => c.classList.toggle("drop-before", j === i && !strip));
    });
    col.addEventListener("dragleave", (e) => {
      if (!col.contains(e.relatedTarget as Node)) {
        col.classList.remove("over");
        list.querySelectorAll(".drop-before").forEach((x) => x.classList.remove("drop-before"));
      }
    });
    col.addEventListener("drop", (e) => {
      e.preventDefault();
      col.classList.remove("over");
      const id = dragging;
      if (!id) return;
      const wasOff = !enabled(ctx.cfg, id);
      if (bucket === "disabled" && !scope.monitor) {
        // Off everywhere: out of every bar and every monitor's layout.
        const base = stripped(ctx.cfg, id);
        ctx.saveMany([...layoutDiffOps(ctx.cfg, base), { path: ["widgets", id, "enabled"], value: false }]);
        return;
      }
      // Switching a widget back on: it goes only where it's dropped.
      const base = wasOff ? stripped(ctx.cfg, id) : ctx.cfg;
      const ls: Record<string, LayoutCfg> = Object.fromEntries(ids.map((s) => [s, scopedLayout(base, s)]));
      const next: Record<string, LayoutCfg> = structuredClone(ls);
      for (const s of ids) {
        for (const side of SIDES) next[s][side] = next[s][side].filter((x) => x !== id);
        next[s].notch = next[s].notch.filter((x) => x !== id);
      }
      if (bucket === "disabled") {
        // Monitor view: just off this monitor's bar (it was taken out above).
        for (const s of ids) if (s !== cur) next[s] = ls[s];
      } else if (elsewhere) {
        next[bucket.slice(8)].center.push(id);
      } else {
        const { i, chips } = indexAt(e.clientY);
        const index = chips.slice(0, i).filter((c) => c.dataset.id !== id).length;
        const side = next[cur][bucket as Side];
        side.splice(Math.min(index, side.length), 0, id);
      }
      const changed = Object.fromEntries(Object.entries(next).filter(([s, l]) => !sameLayout(l, ls[s])));
      // Global view: a widget newly put on (or taken off) a bar is put on (taken off)
      // that bar on monitors with their own layout too; reordering leaves them alone.
      let target = base;
      if (!scope.monitor && Array.isArray(base.monitors)) {
        target = structuredClone(base);
        for (const r of target.monitors) {
          for (const [s, o] of Object.entries<any>(r.surfaces ?? {})) {
            if (!hasLists(o) || !next[s]) continue;
            const was = sideOf(ls[s], id);
            const now = sideOf(next[s], id);
            if (was && !now) for (const k of LISTS) if (Array.isArray(o[k])) o[k] = o[k].filter((x: string) => x !== id);
            if (!was && now && !LISTS.slice(0, 3).some((k) => (o[k] ?? []).includes(id))) {
              o[now] = [...(o[now] ?? []), id];
              if (next[s].notch.includes(id)) o.notch = [...(o.notch ?? []), id];
            }
          }
        }
      }
      ctx.saveMany([
        ...layoutDiffOps(ctx.cfg, target),
        ...(wasOff && bucket !== "disabled" ? [{ path: ["widgets", id, "enabled"], value: true }] : []),
        ...layoutsOps(target, changed),
      ]);
    });
    root.append(col);
  }
  const own = (ctx.cfg.monitors ?? [])
    .filter((r: any) => hasLists(r.surfaces?.[cur]))
    .map((r: any) => r.match);
  const help = scope.monitor
    ? "Drag widgets between sides and bars; Disabled hides a widget on this monitor's bar. The pin shows a widget as a notch on this monitor."
    : "Drag widgets between sides and bars — a widget lives in one place. Disabled switches it off everywhere. A click on a widget opens its settings." +
      (own.length ? ` Adding or removing a widget here also applies to ${own.join(", ")}, which have their own layout of this bar.` : "");
  return $("div", "", root, $("p", "card-help", help));
};

/**
 * v2 -> v3: per-widget notch tables become per-side [notch.<side>] settings, and
 * the single hot-corner position becomes independent switches.
 */
export function migrationOpsV3(cfg: Cfg): Op[] {
  const ops: Op[] = [];
  const L = normalize(cfg.layout);
  for (const [w, settings] of Object.entries<any>(cfg.widgets ?? {})) {
    const n = settings?.notch;
    if (!n || typeof n !== "object") continue;
    const side = sideOf(L, w);
    if (side === "left" || side === "right") {
      if (n.dock !== undefined) ops.push({ path: ["notch", side, "hug"], value: !!n.dock });
      if (typeof n.offset === "number" && n.offset > 0) ops.push({ path: ["notch", side, "offset"], value: n.offset });
      if (n.avoid_windows === false) ops.push({ path: ["notch", side, "avoid_windows"], value: false });
    } else if (side === "center" && n.offset) {
      ops.push({ path: ["notch", "center", "offset"], value: n.offset });
    }
    ops.push({ path: ["widgets", w, "notch"], value: null });
  }
  const t = cfg.timing ?? {};
  if (t.hot_top_left === undefined && t.hot_top_right === undefined && t.hot_top_edge === undefined) {
    const on = t.hot_corner_enabled !== false;
    const pos = t.hot_corner_position ?? "top_left";
    ops.push({ path: ["timing", "hot_top_left"], value: on && pos === "top_left" });
    ops.push({ path: ["timing", "hot_top_right"], value: on && pos === "top_right" });
    ops.push({ path: ["timing", "hot_top_edge"], value: on && pos === "top_edge" });
    if (t.hot_corner_position !== undefined) ops.push({ path: ["timing", "hot_corner_position"], value: null });
    if (t.hot_corner_enabled !== undefined) ops.push({ path: ["timing", "hot_corner_enabled"], value: null });
  }
  return ops;
}

/**
 * Converts a legacy config ([bar] lists + [[notches]] tables + per-monitor
 * notches/bar) to the [layout] model. Returns no edits if already migrated.
 */
export function migrationOps(cfg: Cfg): Op[] {
  if (cfg.layout || cfg.surfaces) return [];
  const L = emptyLayout();
  for (const s of SIDES) L[s] = [...(cfg.bar?.[s] ?? [])];
  const ops: Op[] = [];
  const notchOf: Record<string, string> = {}; // notch id -> widget
  for (const n of cfg.notches ?? []) {
    const side: Side = n.anchor === "left" || n.anchor === "right" ? n.anchor : "center";
    for (const w of n.widgets ?? []) {
      if (!SIDES.some((s) => L[s].includes(w))) L[side].push(w);
      notchOf[n.id] = notchOf[n.id] ?? w;
    }
    const w = (n.widgets ?? [])[0];
    if (!w) continue;
    L.notch.push(w);
    const settings: Record<string, unknown> = {};
    if (n.offset !== undefined) settings.offset = n.offset;
    if (n.width !== undefined) settings.width = n.width;
    if (n.dock) settings.dock = true;
    if (n.avoid_windows === false) settings.avoid_windows = false;
    for (const [k, v] of Object.entries(settings)) ops.push({ path: ["widgets", w, "notch", k], value: v });
  }
  ops.push({ path: ["layout", "left"], value: L.left }, { path: ["layout", "center"], value: L.center });
  ops.push({ path: ["layout", "right"], value: L.right }, { path: ["layout", "notch"], value: L.notch });
  ops.push({ path: ["notches"], value: null });
  for (const s of SIDES) ops.push({ path: ["bar", s], value: null });

  // Monitor rules: legacy notch lists / bar overrides become per-monitor layouts.
  if (Array.isArray(cfg.monitors)) {
    const rules = cfg.monitors.map((r: any) => {
      const out: any = { ...r };
      delete out.notches;
      delete out.bar;
      if (Array.isArray(r.notches) || r.bar) {
        const own = structuredClone(L);
        if (r.bar) for (const s of SIDES) own[s] = [...(r.bar[s] ?? [])];
        const shown = new Set((r.notches ?? Object.keys(notchOf)).map((id: string) => notchOf[id]).filter(Boolean));
        own.notch = L.notch.filter((w) => shown.has(w));
        // A notch widget that isn't shown here shouldn't appear in its bar either.
        for (const w of L.notch) if (!shown.has(w)) for (const s of SIDES) own[s] = own[s].filter((x) => x !== w);
        if (!sameLayout(own, L)) {
          out.layout = own;
          out.custom_layout = true;
        }
      }
      return out;
    });
    ops.push({ path: ["monitors"], value: rules });
  }
  return ops;
}

/**
 * v3 -> v4: the single [layout] becomes the top surface ([surfaces.top]), and
 * monitors' own layouts move into their per-surface settings.
 */
export function migrationOpsV4(cfg: Cfg): Op[] {
  if (cfg.surfaces || !cfg.layout) return [];
  const L = normalize(cfg.layout);
  const ops: Op[] = [
    { path: ["surfaces", "top", "edge"], value: "top" },
    { path: ["surfaces", "top", "mode"], value: "bar" },
    ...LISTS.map((k) => ({ path: ["surfaces", "top", k], value: L[k] })),
    { path: ["layout"], value: null },
  ];
  if (Array.isArray(cfg.monitors)) {
    let changed = false;
    const rules = cfg.monitors.map((r: any) => {
      if (!r.layout && r.custom_layout === undefined && r.follow_unchanged === undefined) return r;
      changed = true;
      const out: any = { ...r };
      const top: any = {};
      if (r.custom_layout !== undefined) top.custom_layout = r.custom_layout;
      if (r.follow_unchanged !== undefined) top.follow_unchanged = r.follow_unchanged;
      if (r.layout) Object.assign(top, normalize(r.layout));
      delete out.layout;
      delete out.custom_layout;
      delete out.follow_unchanged;
      out.surfaces = { ...(r.surfaces ?? {}), top };
      return out;
    });
    if (changed) ops.push({ path: ["monitors"], value: rules });
  }
  return ops;
}

// --- Collapsible ---------------------------------------------------------------

const open = new Set<string>();
/** Opens a dropdown (`page:title`) the next time it's drawn. */
export function openSection(key: string): void {
  open.add(key);
}
export function collapsible(title: string, body: HTMLElement, key: string, icon?: number): HTMLElement {
  const head = $("button", "sub-head", ...(icon ? [glyph(icon)] : []), $("span", "sub-title", title), glyph(0xe70d));
  const box = $("div", `sub${open.has(key) ? " open" : ""}`, head, body);
  box.dataset.key = key;
  head.addEventListener("click", () => {
    box.classList.toggle("open");
    if (box.classList.contains("open")) open.add(key);
    else open.delete(key);
  });
  return box;
}

// --- Clock ---------------------------------------------------------------------

const CLOCK_PRESETS: [string, string][] = [
  ["system", "Follow Windows"],
  ["%H:%M", "24-hour (13:45)"],
  ["%I:%M %p", "12-hour (01:45 PM)"],
  ["%H:%M:%S", "24-hour with seconds"],
  ["custom", "Custom…"],
];

const clockFormat: Editor = (ctx) => {
  const current: string = ctx.cfg.widgets?.clock?.format ?? "%H:%M";
  const isPreset = CLOCK_PRESETS.some(([v]) => v === current && v !== "custom");
  const box = $("div", "stack-tight");
  const pick = select(CLOCK_PRESETS, isPreset ? current : "custom");
  const custom = input(isPreset ? "" : current, "%H:%M");
  const customRow = row(
    "Custom format",
    "%H hours (24h), %I hours (12h), %M minutes, %S seconds, %p AM/PM, %a weekday, %d day, %b month.",
    custom,
  );
  customRow.hidden = isPreset;
  pick.addEventListener("change", () => {
    customRow.hidden = pick.value !== "custom";
    if (pick.value !== "custom") ctx.save(["widgets", "clock", "format"], pick.value);
  });
  custom.addEventListener("change", () => custom.value && ctx.save(["widgets", "clock", "format"], custom.value));
  box.append(row("Time format", undefined, pick), customRow);
  return box;
};

// --- Weather location search ----------------------------------------------------

const weatherLocation: Editor = (ctx) => {
  const w = ctx.cfg.widgets?.weather ?? {};
  const box = $("div", "stack-tight");
  const where = $(
    "div",
    "row-help",
    w.latitude != null ? `${w.location || "Unnamed"} · ${Number(w.latitude).toFixed(3)}, ${Number(w.longitude).toFixed(3)}` : "Not set",
  );
  const q = input("", "Search for a city…");
  const go = $("button", "button", "Search") as HTMLButtonElement;
  const results = $("div", "results");
  const search = async () => {
    const name = q.value.trim();
    if (!name) return;
    results.replaceChildren($("div", "row-help", "Searching…"));
    try {
      const r = await fetch(`https://geocoding-api.open-meteo.com/v1/search?count=6&language=en&name=${encodeURIComponent(name)}`);
      const data = await r.json();
      const hits: any[] = data.results ?? [];
      results.replaceChildren(
        ...(hits.length
          ? hits.map((h) => {
              const label = [h.name, h.admin1, h.country].filter(Boolean).join(", ");
              const b = $("button", "result", label) as HTMLButtonElement;
              b.addEventListener("click", () =>
                ctx.saveMany([
                  { path: ["widgets", "weather", "location"], value: h.name },
                  { path: ["widgets", "weather", "latitude"], value: h.latitude },
                  { path: ["widgets", "weather", "longitude"], value: h.longitude },
                ]),
              );
              return b;
            })
          : [$("div", "row-help", "No places found.")]),
      );
    } catch (e) {
      results.replaceChildren($("div", "row-help", `Search failed: ${e}`));
    }
  };
  go.addEventListener("click", search);
  q.addEventListener("keydown", (e) => e.key === "Enter" && search());
  const label = $("div", "row-text", $("div", "row-label", "Location"), where);
  box.append($("div", "row", label, $("div", "row-control search", q, go)), results);
  return box;
};

// --- PC temps sensor pickers -------------------------------------------------------

const SENSOR_KEYS: [string, string, string[]][] = [
  ["cpu_sensor", "CPU temperature", ["Temperature"]],
  ["gpu_sensor", "GPU temperature", ["Temperature"]],
  ["cpu_load", "CPU load", ["Load"]],
  ["cpu_power", "CPU power", ["Power"]],
  ["cpu_clock", "CPU clock", ["Clock"]],
  ["gpu_load", "GPU load", ["Load"]],
  ["gpu_power", "GPU power", ["Power"]],
  ["vram_used", "VRAM used", ["SmallData", "Data"]],
  ["vram_total", "VRAM total", ["SmallData", "Data"]],
  ["ram_used", "Memory used", ["Data"]],
  ["ram_available", "Memory available", ["Data"]],
];

const tempsSensors: Editor = (ctx) => {
  const box = $("div", "stack-tight", $("div", "row-help", "Loading sensors from LibreHardwareMonitor…"));
  const t = ctx.cfg.widgets?.temps ?? {};
  const url: string = t.url ?? "http://localhost:8085/data.json";
  invoke<any[]>("lhm_sensors", { url })
    .then((sensors) => {
      box.replaceChildren();
      for (const [key, label, types] of SENSOR_KEYS) {
        const options: [string, string][] = sensors
          .filter((s) => types.includes(s.type))
          .map((s) => [s.id, `${s.hardware ?? ""} › ${s.name} (${s.value})`]);
        const current: string = t[key] ?? "";
        if (current && !options.some(([v]) => v === current)) options.unshift([current, `${current} (not found)`]);
        options.unshift(["", "—"]);
        const sel = select(options, current);
        sel.classList.add("wide");
        sel.addEventListener("change", () => ctx.save(["widgets", "temps", key], sel.value || null));
        box.append(row(label, undefined, sel));
      }
    })
    .catch((e) => box.replaceChildren($("div", "row-help", `Couldn't reach LibreHardwareMonitor at ${url}: ${e}`)));
  return box;
};

// --- Menu entries -----------------------------------------------------------------

const ACTIONS: [string, string][] = [
  ["open", "Open folder or file"],
  ["settings", "StatusNotch settings"],
  ["lock", "Lock"],
  ["sleep", "Sleep"],
  ["hibernate", "Hibernate"],
  ["sign_out", "Sign out"],
  ["restart", "Restart"],
  ["shutdown", "Shut down"],
  ["open_config", "Open config file"],
];
const ICONS: [number, string][] = [
  [0xe8b7, "Folder"], [0xe838, "Folder open"], [0xe7f4, "This PC"], [0xe896, "Downloads"],
  [0xe898, "Upload"], [0xe8a5, "Document"], [0xe7c3, "Page"], [0xeb9f, "Picture"],
  [0xe8f1, "Library"], [0xe74d, "Trash"], [0xe80f, "Home"], [0xe71d, "Apps"],
  [0xe721, "Search"], [0xe713, "Settings"], [0xe771, "Personalize"], [0xe790, "Color"],
  [0xe72e, "Lock"], [0xe708, "Moon"], [0xe706, "Brightness"], [0xea80, "Lightbulb"],
  [0xe777, "Restart"], [0xe7e8, "Power"], [0xe8f8, "Sign out"], [0xe72c, "Refresh"],
  [0xe895, "Sync"], [0xe774, "Globe"], [0xe71b, "Link"], [0xe753, "Cloud"],
  [0xe715, "Mail"], [0xe8bd, "Chat"], [0xe717, "Phone"], [0xe716, "People"],
  [0xe787, "Calendar"], [0xe823, "Clock"], [0xe8d6, "Music"], [0xe714, "Video"],
  [0xe722, "Camera"], [0xe7fc, "Game"], [0xe7f6, "Headphones"], [0xe720, "Microphone"],
  [0xe767, "Volume"], [0xe74f, "Mute"], [0xe701, "Wi-Fi"], [0xe702, "Bluetooth"],
  [0xe709, "Airplane"], [0xe765, "Keyboard"], [0xe962, "Mouse"], [0xe943, "Code"],
  [0xe756, "Terminal"], [0xe8ef, "Calculator"], [0xe7bf, "Shopping"], [0xe734, "Star"],
  [0xeb51, "Heart"], [0xe718, "Pin"], [0xe7c1, "Flag"], [0xe8ec, "Tag"],
  [0xe70f, "Edit"], [0xe8c8, "Copy"], [0xe74e, "Save"], [0xe749, "Print"],
  [0xe72d, "Share"], [0xea18, "Shield"], [0xe7ef, "Admin"], [0xe946, "Info"],
  [0xe897, "Help"], [0xe7ba, "Warning"],
];

/**
 * An icon button that opens a grid of icons below `row` (names only as tooltips).
 * Returns the button and the grid; the grid goes right after the row it belongs
 * to (inline, so a collapsible section can't clip it).
 */
function iconPicker(current: string, onPick: (glyph: string) => void): [HTMLElement, HTMLElement] {
  const btn = $("button", "icon-picker", $("span", "icon", current || String.fromCharCode(0xe8b7)));
  btn.title = "Choose an icon";
  const grid = $("div", "icon-grid");
  grid.hidden = true;
  for (const [code, name] of ICONS) {
    const ch = String.fromCharCode(code);
    const cell = $("button", `icon-cell${ch === current ? " on" : ""}`, glyph(code));
    cell.title = name;
    cell.addEventListener("click", () => {
      grid.querySelectorAll(".icon-cell.on").forEach((c) => c.classList.remove("on"));
      cell.classList.add("on");
      btn.replaceChildren($("span", "icon", ch));
      grid.hidden = true;
      onPick(ch);
    });
    grid.append(cell);
  }
  btn.addEventListener("click", () => {
    const open = grid.hidden;
    document.querySelectorAll<HTMLElement>(".icon-grid").forEach((g) => (g.hidden = true));
    grid.hidden = !open;
  });
  return [btn, grid];
}
const DEFAULT_ICON: Record<string, number> = {
  open: 0xe8b7,
  settings: 0xe713,
  lock: 0xe72e,
  sleep: 0xe708,
  hibernate: 0xe708,
  sign_out: 0xe8f8,
  restart: 0xe777,
  shutdown: 0xe7e8,
  open_config: 0xe7c3,
};
const DESTRUCTIVE = new Set(["restart", "shutdown", "sign_out"]);

/** Editor for a widget's `items` (menu entries, buttons). */
const itemsEditor = (widget: string, separators = true): Editor => (ctx) => {
  const items: any[] = structuredClone(ctx.cfg.widgets?.[widget]?.items ?? []);
  const save = () => ctx.save(["widgets", widget, "items"], items);
  const list = $("div", "menu-editor");

  const render = () => {
    list.replaceChildren();
    items.forEach((it, i) => {
      const controls = $("div", "menu-row-actions");
      controls.append(
        iconButton(0xe70e, "Move up", () => {
          if (i === 0) return;
          [items[i - 1], items[i]] = [items[i], items[i - 1]];
          render();
          save();
        }),
        iconButton(0xe70d, "Move down", () => {
          if (i === items.length - 1) return;
          [items[i + 1], items[i]] = [items[i], items[i + 1]];
          render();
          save();
        }),
        iconButton(0xe74d, "Remove", () => {
          items.splice(i, 1);
          render();
          save();
        }),
      );
      if (it.separator) {
        list.append($("div", "menu-row sep-row", $("div", "sep-line"), controls));
        return;
      }
      const [iconBtn, iconGrid] = iconPicker(it.icon ?? String.fromCharCode(DEFAULT_ICON[it.action] ?? 0xe8b7), (ch) => {
        it.icon = ch;
        save();
      });
      const label = input(it.label ?? "", "Label");
      label.addEventListener("change", () => {
        it.label = label.value;
        save();
      });
      const action = select(ACTIONS, it.action ?? "open");
      const target = input(it.target ?? "", "C:/path or shell:Downloads");
      target.hidden = (it.action ?? "open") !== "open";
      target.addEventListener("change", () => {
        it.target = target.value;
        save();
      });
      action.addEventListener("change", () => {
        it.action = action.value;
        target.hidden = action.value !== "open";
        if (action.value !== "open") delete it.target;
        if (DESTRUCTIVE.has(action.value)) it.confirm = true;
        else delete it.confirm;
        it.icon = String.fromCharCode(DEFAULT_ICON[action.value] ?? 0xe8b7);
        iconBtn.replaceChildren($("span", "icon", it.icon));
        if (!it.label) it.label = ACTIONS.find(([v]) => v === action.value)?.[1] ?? "";
        label.value = it.label;
        save();
      });
      list.append($("div", "menu-row", iconBtn, label, action, target, controls), iconGrid);
    });
  };
  render();

  const add = (item: any) => () => {
    items.push(item);
    render();
    save();
  };
  const buttons = $(
    "div",
    "button-row",
    $("button", "button", "Add folder"),
    $("button", "button", "Add action"),
    $("button", "button", "Add separator"),
  );
  const [bFolder, bAction, bSep] = [...buttons.children] as HTMLButtonElement[];
  bFolder.addEventListener("click", add({ label: "Folder", icon: String.fromCharCode(0xe8b7), action: "open", target: "C:/" }));
  bAction.addEventListener("click", add({ label: "Lock", icon: String.fromCharCode(0xe72e), action: "lock" }));
  bSep.addEventListener("click", add({ separator: true }));
  bSep.hidden = !separators;
  return $("div", "stack-tight", list, buttons, $("p", "card-help", "Restart, shut down and sign out ask for a second click."));
};

const menuItems = itemsEditor("menu");

/** Every Buttons widget (`buttons`, `buttons#2`, ...): its buttons, notch, add / remove. */
const buttonsWidgets: Editor = (ctx) => {
  const ids = catalog(ctx.cfg)
    .filter((w) => typeOf(w.id) === "buttons")
    .map((w) => w.id);
  const box = $("div", "stack");
  for (const id of ids) {
    const label = catalog(ctx.cfg).find((w) => w.id === id)!.label;
    const head = $("div", "buttons-head", $("div", "row-label", label));
    if (id !== "buttons") {
      const remove = $("button", "button", "Remove") as HTMLButtonElement;
      let armed = false;
      remove.addEventListener("click", () => {
        if (!armed) {
          armed = true;
          remove.textContent = "Click again to remove";
          remove.classList.add("danger");
          return;
        }
        // Off every bar (globally and per monitor), then its settings.
        const ops: Op[] = [];
        for (const [s, sc] of Object.entries<any>(ctx.cfg.surfaces ?? {})) {
          for (const k of LISTS) {
            if ((sc?.[k] ?? []).includes(id)) ops.push({ path: ["surfaces", s, k], value: sc[k].filter((x: string) => x !== id) });
          }
        }
        if (Array.isArray(ctx.cfg.monitors)) {
          const rules = structuredClone(ctx.cfg.monitors);
          let changed = false;
          for (const r of rules) {
            for (const o of Object.values<any>(r.surfaces ?? {})) {
              for (const k of LISTS) {
                if (Array.isArray(o?.[k]) && o[k].includes(id)) {
                  o[k] = o[k].filter((x: string) => x !== id);
                  changed = true;
                }
              }
            }
          }
          if (changed) ops.push({ path: ["monitors"], value: rules });
        }
        ops.push({ path: ["widgets", id], value: null });
        ctx.saveMany(ops);
      });
      head.append(remove);
    }
    box.append($("div", "buttons-card", head, itemsEditor(id, false)(ctx)));
  }
  const add = $("button", "button", "Add another Buttons widget") as HTMLButtonElement;
  add.addEventListener("click", () => {
    const n = Math.max(1, ...ids.map((id) => Number(id.split("#")[1] ?? 1))) + 1;
    ctx.saveMany([
      {
        path: ["widgets", `buttons#${n}`, "items"],
        value: [{ label: "Settings", icon: String.fromCharCode(0xe713), action: "settings" }],
      },
    ]);
  });
  return $(
    "div",
    "stack",
    box,
    $("div", "button-row", add),
    $("p", "card-help", "Each Buttons widget is placed on its own (Placement above): e.g. a power button in the top bar and a settings button in the dock."),
  );
};


// --- Presets -------------------------------------------------------------------

interface Preset {
  name: string;
  modified: number;
  current: boolean;
}

/** First click arms the button (red, with a hint), a second within 3s runs it. */
function confirmClick(b: HTMLButtonElement, hint: string, run: () => void): void {
  let timer = 0;
  const title = b.title;
  b.addEventListener("click", () => {
    if (!b.classList.contains("danger")) {
      b.classList.add("danger");
      b.title = hint;
      timer = window.setTimeout(() => {
        b.classList.remove("danger");
        b.title = title;
      }, 3000);
      return;
    }
    clearTimeout(timer);
    b.classList.remove("danger");
    b.title = title;
    run();
  });
}

/** Presets page: save the current setup under a name, apply, update, rename, delete. */
function presets(ctx: EditorCtx): HTMLElement {
  const list = $("div", "preset-list");
  let names: string[] = [];
  const fail = (e: unknown) => ctx.toast(String(e), true);

  const refresh = async () => {
    const all = await invoke<Preset[]>("presets_list").catch((e) => (fail(e), [] as Preset[]));
    names = all.map((p) => p.name.toLowerCase());
    list.replaceChildren(
      ...(all.length
        ? all.map(item)
        : [$("div", "row preset-empty", "No presets yet. Save your current setup to make the first one.")]),
    );
  };

  const item = (p: Preset): HTMLElement => {
    const label = $("div", "row-label", p.name);
    if (p.current) label.append($("span", "preset-tag", "In use"));
    const saved = new Date(p.modified).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
    const text = $("div", "row-text", label, $("div", "row-help", `Saved ${saved}`));

    const apply = $("button", "button", p.current ? "Applied" : "Apply") as HTMLButtonElement;
    apply.disabled = p.current;
    apply.addEventListener("click", async () => {
      try {
        await invoke("preset_apply", { name: p.name });
        await ctx.reload();
        ctx.toast(`Applied “${p.name}”`);
      } catch (e) {
        fail(e);
      }
    });

    const update = iconButton(0xe74e, "Save the current setup into this preset", () => {});
    confirmClick(update, "Click again to replace it with the current setup", async () => {
      try {
        await invoke("preset_save", { name: p.name });
        ctx.toast(`Updated “${p.name}”`);
        await refresh();
      } catch (e) {
        fail(e);
      }
    });

    const rename = iconButton(0xe8ac, "Rename", () => {
      const field = input(p.name, "Preset name");
      field.classList.add("preset-rename");
      let finished = false;
      const done = async (commit: boolean) => {
        // Enter and the blur that follows must not both run.
        if (finished) return;
        finished = true;
        const to = field.value.trim();
        if (!commit || !to || to === p.name) return refresh();
        try {
          await invoke("preset_rename", { from: p.name, to });
        } catch (e) {
          fail(e);
        }
        await refresh();
      };
      field.addEventListener("keydown", (e) => {
        if (e.key === "Enter") done(true);
        if (e.key === "Escape") done(false);
      });
      field.addEventListener("blur", () => done(true), { once: true });
      label.replaceWith(field);
      field.focus();
      field.select();
    });

    const remove = iconButton(0xe74d, "Delete", () => {});
    confirmClick(remove, "Click again to delete", async () => {
      try {
        await invoke("preset_delete", { name: p.name });
        ctx.toast(`Deleted “${p.name}”`);
      } catch (e) {
        fail(e);
      }
      await refresh();
    });

    return $("div", "row", text, $("div", "row-control preset-actions", update, rename, remove, apply));
  };

  // Save the current setup.
  const name = input("", "Name, e.g. Gaming or Work");
  const save = $("button", "button", "Save") as HTMLButtonElement;
  const disarm = () => {
    save.textContent = "Save";
    save.classList.remove("danger");
  };
  const doSave = async () => {
    const n = name.value.trim();
    if (!n) return name.focus();
    // Replacing an existing preset takes a second click.
    if (names.includes(n.toLowerCase()) && !save.classList.contains("danger")) {
      save.textContent = "Replace?";
      save.classList.add("danger");
      return;
    }
    try {
      await invoke("preset_save", { name: n });
      ctx.toast(`Saved “${n}”`);
      name.value = "";
      disarm();
      await refresh();
    } catch (e) {
      fail(e);
    }
  };
  name.addEventListener("input", disarm);
  name.addEventListener("keydown", (e) => {
    if (e.key === "Enter") doSave();
  });
  save.addEventListener("click", doSave);
  const saveRow = row(
    "Save current setup",
    "Everything in settings: layout, notches, looks, widgets and monitors.",
    $("div", "preset-save", name, save),
  );

  refresh();
  return $("div", "presets", saveRow, list);
}

/** Quick settings tiles: the chosen ones in order (move, remove), the rest to add. */
// --- Windows setup for notifications --------------------------------------------------

/**
 * How to set Windows up so StatusNotch shows the notifications and Windows
 * doesn't (it has to keep storing them: that's where they're read from).
 */
const windowsGuide: Editor = () => {
  const status = $("span", "guide-status", "…");
  invoke<boolean>("notifications_windows_on")
    .then((on) => {
      status.textContent = on ? "On" : "Off: turn on";
      status.classList.toggle("bad", !on);
    })
    .catch(() => status.remove());
  const step = (n: string, title: string, text: string, ...extra: HTMLElement[]) =>
    $("div", "guide-step", $("span", "guide-num", n), $("div", "guide-text", $("div", "guide-title", title, ...extra), $("div", "guide-help", text)));
  const open = $("button", "button", glyph(0xe713), "Open Windows notification settings");
  open.addEventListener("click", () => invoke("run_action", { action: "open", target: "ms-settings:notifications" }).catch(console.error));
  return $(
    "div",
    "guide",
    $("div", "guide-head", glyph(0xe946), $("span", "", "Set up Windows for this")),
    step("1", "Notifications on", "StatusNotch reads notifications from Windows. While they're off, Windows keeps none, so there's nothing to show.", status),
    step(
      "2",
      "Do not disturb on",
      "Windows keeps every notification but shows no pop-ups of its own; StatusNotch shows them instead. Apps on its priority list (calls, reminders, alarms) still get Windows' pop-ups.",
    ),
    step(
      "",
      "Or: banners off, per app",
      "Instead of Do not disturb: click an app on that page and switch off \"Show notification banners\". Leave \"Show notifications in notification center\" on.",
    ),
    $("div", "button-row", open),
  );
};

// --- OSD: where each pop-up appears ------------------------------------------------------

const OSD_SPOTS: [string, string][] = [
  ["top-left", "Top bar · left corner"],
  ["top-center", "Top bar · center"],
  ["top-right", "Top bar · right corner"],
  ["bottom-left", "Bottom bar · left corner"],
  ["bottom-center", "Bottom bar · center"],
  ["bottom-right", "Bottom bar · right corner"],
];

/**
 * A small screen with six notches to click (three on top, three at the bottom),
 * plus chips for the choices that aren't a place on it. `inherited`: the spot
 * "Like the default" means, outlined while that's chosen.
 */
function spotMap(value: string, extras: [string, string][], inherited: string, onPick: (v: string) => void): HTMLElement {
  const screen = $("div", "spot-map");
  const chips = $("div", "scope-bar");
  const paint = (v: string) => {
    for (const b of screen.querySelectorAll<HTMLElement>(".spot-node")) {
      b.classList.toggle("active", b.dataset.spot === v);
      b.classList.toggle("inherited", v === "" && b.dataset.spot === inherited);
    }
    for (const c of chips.querySelectorAll<HTMLElement>(".scope-chip")) c.classList.toggle("active", c.dataset.spot === v);
  };
  const pick = (v: string) => {
    paint(v);
    onPick(v);
  };
  for (const [spot, label] of OSD_SPOTS) {
    const b = $("button", `spot-node spot-${spot}`);
    b.dataset.spot = spot;
    b.title = label;
    b.addEventListener("click", () => pick(spot));
    screen.append(b);
  }
  for (const [spot, label] of extras) {
    const c = $("button", "scope-chip", label);
    c.dataset.spot = spot;
    c.addEventListener("click", () => pick(spot));
    chips.append(c);
  }
  paint(value);
  return $("div", "spot-picker", screen, ...(extras.length ? [chips] : []));
}

/**
 * Screen and spot of a pop-up ("osdPlace:volume" etc.; "osdPlace" alone: the
 * default the others follow). Writes [osd] <kind>_screen / <kind>_spot, or
 * screen / spot.
 */
const osdPlace: Editor = (ctx) => {
  const kind = ctx.arg ?? "";
  const key = (what: "screen" | "spot") => (kind ? `${kind}_${what}` : what);
  const osd = ctx.cfg.osd ?? {};
  const like: [string, string][] = kind ? [["", "Like the default"]] : [];
  const screen = select([...like, ["cursor", "The screen with the cursor"], ["main", "Main screen"], ["all", "All screens"], ["others", "All but the main screen"]], "");
  screen.value = osd[key("screen")] ?? (kind ? "" : "cursor");
  // The monitors by name, once known (a saved name is kept even if it's not connected).
  loadMonitors().then((list) => {
    for (const m of list) screen.append(new Option(m.name, m.name));
    const saved = osd[key("screen")];
    if (saved && ![...screen.options].some((o) => o.value === saved)) screen.append(new Option(saved, saved));
    screen.value = saved ?? (kind ? "" : "cursor");
  });
  const extras: [string, string][] = [...like, ...(kind === "keyboard" ? ([["widget", "At the language widget"]] as [string, string][]) : [])];
  const spot = spotMap(osd[key("spot")] ?? (kind === "keyboard" ? "widget" : kind ? "" : "top-center"), extras, osd.spot ?? "top-center", (v) =>
    ctx.save(["osd", key("spot")], v || null),
  );
  screen.addEventListener("change", () => ctx.save(["osd", key("screen")], screen.value || null));
  return $(
    "div",
    "stack-tight",
    row("Screen", kind ? undefined : "Where pop-ups appear, unless one says otherwise below.", screen),
    row("Spot", kind ? "Which notch it grows out of." : "Which notch grows: a bar's center, or its corner notch (a bar without one there uses its center).", spot),
  );
};

// --- Disk usage -------------------------------------------------------------------------

/** Which drives the disk widget shows: chips to switch on / off (none chosen = all). */
const diskPicker: Editor = (ctx) => {
  const saved = ctx.cfg.widgets?.disk?.disks;
  const chosen = new Set<string>(Array.isArray(saved) ? saved : []);
  const chips = $("div", "tile-row", $("p", "card-help", "Looking for drives…"));
  invoke<{ id: string; label: string; total: number }[]>("disk_list")
    .then((disks) => {
      chips.replaceChildren(
        ...disks.map((d) => {
          // None chosen means all: show them all as on.
          const on = !chosen.size || chosen.has(d.id);
          const gb = Math.round(d.total / 1024 ** 3);
          const chip = $("div", `tile-chip toggle${on ? "" : " add"}`, glyph(0xeda2), $("span", "", `${d.id} ${d.label || "Local disk"} · ${gb} GB`));
          chip.addEventListener("click", () => {
            const next = new Set(chosen.size ? chosen : disks.map((x) => x.id));
            if (next.has(d.id)) next.delete(d.id);
            else next.add(d.id);
            // All of them again: back to "all" (new drives show up too).
            const value = next.size === disks.length ? null : disks.map((x) => x.id).filter((x) => next.has(x));
            ctx.saveMany([{ path: ["widgets", "disk", "disks"], value }]);
          });
          return chip;
        }),
      );
    })
    .catch(() => chips.replaceChildren($("p", "card-help", "Couldn't list the drives.")));
  return $("div", "stack-tight", row("Drives", "Which drives it shows (all of them when none is off).", $("span")), chips);
};

// --- Launcher power buttons ----------------------------------------------------------

/** Which power buttons the launcher shows: chips to switch on / off (bright = on). */
const launcherPower: Editor = (ctx) => {
  const saved = ctx.cfg.widgets?.launcher?.power;
  const on = new Set<string>(Array.isArray(saved) ? saved : []);
  const chips = $("div", "tile-row");
  for (const [id, [label, code]] of Object.entries(POWER_ACTIONS)) {
    const chip = $("div", `tile-chip toggle${on.has(id) ? "" : " add"}`, glyph(code), $("span", "", label));
    chip.title = on.has(id) ? "Shown: click to hide" : "Click to show";
    chip.addEventListener("click", () => {
      if (on.has(id)) on.delete(id);
      else on.add(id);
      // Saved in display order.
      ctx.saveMany([{ path: ["widgets", "launcher", "power"], value: Object.keys(POWER_ACTIONS).filter((p) => on.has(p)) }]);
    });
    chips.append(chip);
  }
  return $(
    "div",
    "stack-tight",
    row("Power buttons", "Shown at the end of the launcher, like a start menu's. Restart, shut down and sign out need a second click.", $("span")),
    chips,
  );
};

const quickTiles: Editor = (ctx) => {
  const saved = ctx.cfg.widgets?.quick?.tiles;
  const chosen: string[] = (Array.isArray(saved) ? saved.filter((t: string) => t in QUICK_TILES) : [...DEFAULT_TILES]).slice(0, MAX_TILES);
  const save = () => ctx.saveMany([{ path: ["widgets", "quick", "tiles"], value: chosen }]);
  const tile = (id: string, ...extra: HTMLElement[]) => {
    const [label, code] = QUICK_TILES[id];
    return $("div", "tile-chip", glyph(code), $("span", "", label), ...extra);
  };
  const picked = $("div", "tile-row");
  chosen.forEach((id, i) => {
    picked.append(
      tile(
        id,
        iconButton(0xe76b, "Move left", () => {
          if (i === 0) return;
          [chosen[i - 1], chosen[i]] = [chosen[i], chosen[i - 1]];
          save();
        }),
        iconButton(0xe76c, "Move right", () => {
          if (i === chosen.length - 1) return;
          [chosen[i + 1], chosen[i]] = [chosen[i], chosen[i + 1]];
          save();
        }),
        iconButton(0xe711, "Remove", () => {
          chosen.splice(i, 1);
          save();
        }),
      ),
    );
  });
  if (!chosen.length) picked.append($("p", "card-help", "No tiles: the panel shows none."));
  const full = chosen.length >= MAX_TILES;
  const rest = $("div", "tile-row");
  for (const id of Object.keys(QUICK_TILES).filter((t) => !chosen.includes(t))) {
    const add = tile(id);
    add.classList.add("add");
    if (full) add.classList.add("full");
    add.title = full ? `Up to ${MAX_TILES} tiles: remove one first` : "Add";
    add.addEventListener("click", () => {
      if (chosen.length >= MAX_TILES) return;
      chosen.push(id);
      save();
    });
    rest.append(add);
  }
  return $(
    "div",
    "stack-tight",
    row("Tiles", `Shortcuts in one row at the bottom of the panel, in this order (up to ${MAX_TILES}; three or fewer show as slim pills).`, $("span")),
    picked,
    ...(rest.children.length ? [$("p", "card-help", full ? `Add (remove one first: ${MAX_TILES} is the most):` : "Add:"), rest] : []),
  );
};

/** Quick settings: what its icon shows, and a custom icon to pick. */
const quickIcon: Editor = (ctx) => {
  const o = ctx.cfg.widgets?.quick ?? {};
  const mode = typeof o.icon === "string" ? o.icon : "auto";
  const sel = select(
    [
      ["auto", "Battery on laptops, else a settings icon"],
      ["battery", "Battery"],
      ["settings", "Settings icon"],
      ["volume", "Volume"],
      ["network", "Network"],
      ["custom", "Custom icon"],
    ],
    mode,
  );
  sel.addEventListener("change", () => {
    ctx.save(["widgets", "quick", "icon"], sel.value);
    ctx.rerender();
  });
  const box = $("div", "stack-tight", row("Shows", undefined, sel));
  if (mode === "custom") {
    const [btn, grid] = iconPicker(typeof o.custom_icon === "string" ? o.custom_icon : String.fromCharCode(0xe713), (ch) =>
      ctx.save(["widgets", "quick", "custom_icon"], ch),
    );
    box.append(row("Icon", undefined, btn), grid);
  }
  return box;
};

export const editors: Record<string, Editor> = {
  quickIcon,
  quickTiles,
  launcherPower,
  diskPicker,
  osdPlace,
  windowsGuide,
  presets,
  surfaceManager,
  surfaceStyle,
  buttonsWidgets,
  scopeSelector,
  monitorToggles,
  placement,
  clockFormat,
  weatherLocation,
  tempsSensors,
  menuItems,
};
