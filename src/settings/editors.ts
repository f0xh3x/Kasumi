// Structured editors for the settings window: things richer than a single
// control (widget placement, notches, menu entries, monitors, lookups).
import { invoke } from "@tauri-apps/api/core";
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
}

type Editor = (ctx: EditorCtx) => HTMLElement;

// --- DOM helpers -------------------------------------------------------------

const $ = <K extends keyof HTMLElementTagNameMap>(tag: K, cls?: string, ...kids: (Node | string)[]) => {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  e.append(...kids);
  return e;
};
const glyph = (code: number) => $("span", "icon", String.fromCharCode(code));

function input(value: string, placeholder = "", cls = "input"): HTMLInputElement {
  const i = $("input", cls) as HTMLInputElement;
  i.value = value;
  i.placeholder = placeholder;
  return i;
}

function select(options: [string, string][], value: string): HTMLSelectElement {
  const s = $("select", "select") as HTMLSelectElement;
  for (const [v, label] of options) s.append(new Option(label, v));
  s.value = value;
  return s;
}

function iconButton(code: number, title: string, onClick: () => void): HTMLButtonElement {
  const b = $("button", "icon-button", glyph(code)) as HTMLButtonElement;
  b.title = title;
  b.addEventListener("click", onClick);
  return b;
}

function sw(on: boolean, onChange: (on: boolean) => void): HTMLButtonElement {
  const b = $("button", `switch${on ? " on" : ""}`) as HTMLButtonElement;
  b.addEventListener("click", () => {
    on = !on;
    b.classList.toggle("on", on);
    onChange(on);
  });
  return b;
}

function row(label: string, help: string | undefined, control: HTMLElement): HTMLElement {
  const text = $("div", "row-text", $("div", "row-label", label));
  if (help) text.append($("div", "row-help", help));
  return $("div", "row", text, $("div", "row-control", control));
}

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
];

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

/** The surface ids in the config (configs from before surfaces: just the top bar). */
export const surfaceIds = (cfg: Cfg): string[] => (cfg.surfaces ? Object.keys(cfg.surfaces) : ["top"]);

/** A surface's global layout. */
export const surfaceLayout = (cfg: Cfg, id: string): LayoutCfg =>
  cfg.surfaces ? normalize(cfg.surfaces[id]) : id === "top" ? normalize(cfg.layout) : emptyLayout();

/** Which part of the Widgets page is being edited: a surface, globally or for one monitor. */
export const scope: { monitor: string | null; surface: string } = { monitor: null, surface: "top" };

export const globalLayout = (cfg: Cfg): LayoutCfg => surfaceLayout(cfg, scope.surface);
let monitorsCache: MonitorInfo[] | null = null;

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

const currentLayout = (cfg: Cfg) => (scope.monitor ? monitorLayout(cfg, scope.monitor) : globalLayout(cfg));

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
    for (const m of list) chip(m.primary ? `${m.name} · primary` : m.name, m.name, 0xe7f4);
  };
  if (monitorsCache) draw(monitorsCache);
  else
    invoke<MonitorInfo[]>("monitors_list").then((list) => {
      monitorsCache = list;
      draw(list);
    });
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
    const b = $("button", `scope-chip${scope.surface === id ? " active" : ""}`, glyph(surfaceIcon(s)), surfaceName(id, s));
    b.addEventListener("click", () => {
      scope.surface = id;
      ctx.rerender();
    });
    surfaces.append(b);
  }
  return $("div", "stack-tight", surfaces, bar, $("p", "card-help", help));
};

/** Adds a dock (or another bar), or removes the selected one. */
const surfaceManager: Editor = (ctx) => {
  const ids = surfaceIds(ctx.cfg);
  const box = $("div", "stack-tight");
  const cur = scope.surface;
  const s = ctx.cfg.surfaces?.[cur] ?? {};
  box.append(
    row(
      `Show the ${surfaceName(cur, s).toLowerCase()}`,
      undefined,
      sw(s.enabled !== false, (on) => ctx.saveMany([{ path: ["surfaces", cur, "enabled"], value: on }])),
    ),
  );
  const actions = $("div", "preset-actions");
  if (!ids.includes("bottom")) {
    const add = $("button", "button", "Add a dock") as HTMLButtonElement;
    add.addEventListener("click", async () => {
      await ctx.saveMany([
        { path: ["surfaces", "bottom", "edge"], value: "bottom" },
        { path: ["surfaces", "bottom", "mode"], value: "dock" },
        { path: ["surfaces", "bottom", "rest"], value: "small" },
        { path: ["surfaces", "bottom", "open"], value: "medium" },
        ...LISTS.map((k) => ({ path: ["surfaces", "bottom", k], value: [] as string[] })),
      ]);
      scope.surface = "bottom";
      ctx.rerender();
    });
    actions.append(add);
  }
  if (cur !== "top" && ids.length > 1) {
    const remove = $("button", "button", "Remove") as HTMLButtonElement;
    let armed = false;
    remove.addEventListener("click", async () => {
      if (!armed) {
        armed = true;
        remove.textContent = "Click again to remove";
        remove.classList.add("danger");
        return;
      }
      await ctx.saveMany([{ path: ["surfaces", cur], value: null }]);
      scope.surface = "top";
      ctx.rerender();
    });
    actions.append(remove);
  }
  if (actions.children.length) box.append(row("Bars and docks", "A dock sits at the bottom by default; drag widgets into it below.", actions));
  return box;
};

/** "Top bar", "Dock", ... for a surface. */
export function surfaceName(id: string, s: any): string {
  const edge = s?.edge ?? (id === "bottom" ? "bottom" : "top");
  const kind = s?.mode === "dock" ? "dock" : "bar";
  if (id === "top" || id === "bottom") return `${edge === "top" ? "Top" : "Bottom"} ${kind}`;
  return id;
}
const surfaceIcon = (s: any) => (s?.edge === "bottom" ? 0xe70d : 0xe70e); // chevron down / up

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
    ...ids.filter((s) => s !== cur).map((s): [Bucket, string] => [`surface:${s}`, `In the ${surfaceName(s, ctx.cfg.surfaces?.[s]).toLowerCase()}`]),
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

  for (const [bucket, title] of columns) {
    const elsewhere = bucket.startsWith("surface:");
    const list = $("div", "bucket-list");
    for (const id of listOf(bucket)) {
      const w = cat.find((x) => x.id === id)!;
      const chip = $("div", "chip", glyph(w.icon), w.label);
      const notchIn = elsewhere ? layouts[bucket.slice(8)] : L;
      if (bucket !== "disabled" && notchIn.notch.includes(id)) chip.append($("span", "chip-badge", "notch"));
      chip.draggable = true;
      chip.dataset.id = id;
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
      ctx.saveMany([
        ...layoutDiffOps(ctx.cfg, base),
        ...(wasOff && bucket !== "disabled" ? [{ path: ["widgets", id, "enabled"], value: true }] : []),
        ...layoutsOps(base, changed),
      ]);
    });
    root.append(col);
  }
  const help = scope.monitor
    ? "Drag widgets between sides and bars; Disabled hides a widget on this monitor's bar."
    : "Drag widgets between sides and bars — a widget lives in one place. Disabled takes it off this bar (and switches it off when it's nowhere else). Widgets marked “notch” show as their own notch on their side.";
  return $("div", "", root, $("p", "card-help", help));
};

/** Per-monitor: which widgets show as their own notch here. */
const monitorNotches: Editor = (ctx) => {
  const L = currentLayout(ctx.cfg);
  const box = $("div", "stack-tight");
  for (const w of catalog(ctx.cfg)) {
    if (!enabled(ctx.cfg, w.id) || !sideOf(L, w.id)) continue;
    box.append(
      row(
        `${w.label}`,
        undefined,
        sw(L.notch.includes(w.id), (on) => {
          const next = structuredClone(L);
          next.notch = on ? [...next.notch, w.id] : next.notch.filter((x) => x !== w.id);
          ctx.saveMany(layoutOps(ctx.cfg, next));
        }),
      ),
    );
  }
  if (!box.children.length) box.append($("p", "card-help", "No widgets are placed on this monitor."));
  return box;
};

/** In a widget's own settings: whether it shows as (part of) a notch on its side. */
export function widgetNotch(widget: string): Editor {
  return (ctx) => {
    // The surface it's placed on (globally).
    const id = surfaceIds(ctx.cfg).find((s) => sideOf(surfaceLayout(ctx.cfg, s), widget)) ?? scope.surface;
    const L = surfaceLayout(ctx.cfg, id);
    const side = sideOf(L, widget);
    const has = L.notch.includes(widget);
    const where = surfaceIds(ctx.cfg).length > 1 ? ` of the ${surfaceName(id, ctx.cfg.surfaces?.[id]).toLowerCase()}` : "";
    return row(
      "Own notch",
      side
        ? `Shows in the ${side} notch${where} while the bar is closed. Notch position and corner settings are under Notches.`
        : "Place the widget on a side first (Placement above).",
      sw(has, (on) => {
        const notch = on ? [...L.notch, widget] : L.notch.filter((x) => x !== widget);
        ctx.saveMany(ctx.cfg.surfaces ? [{ path: ["surfaces", id, "notch"], value: notch }] : [{ path: ["layout", "notch"], value: notch }]);
      }),
    );
  };
}

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
export function collapsible(title: string, body: HTMLElement, key: string, icon?: number): HTMLElement {
  const head = $("button", "sub-head", ...(icon ? [glyph(icon)] : []), $("span", "sub-title", title), glyph(0xe70d));
  const box = $("div", `sub${open.has(key) ? " open" : ""}`, head, body);
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
  [0xe8b7, "Folder"],
  [0xe7f4, "This PC"],
  [0xe896, "Downloads"],
  [0xe8a5, "Document"],
  [0xeb9f, "Picture"],
  [0xe74d, "Trash"],
  [0xe713, "Settings"],
  [0xe72e, "Lock"],
  [0xe708, "Moon"],
  [0xe777, "Restart"],
  [0xe7e8, "Power"],
  [0xe8f8, "Sign out"],
  [0xe7c3, "Page"],
];
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
      const iconSel = select(
        ICONS.map(([c, name]) => [String.fromCharCode(c), `${String.fromCharCode(c)}  ${name}`]),
        it.icon ?? String.fromCharCode(DEFAULT_ICON[it.action] ?? 0xe8b7),
      );
      iconSel.classList.add("icon-select");
      iconSel.addEventListener("change", () => {
        it.icon = iconSel.value;
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
        iconSel.value = it.icon;
        if (!it.label) it.label = ACTIONS.find(([v]) => v === action.value)?.[1] ?? "";
        label.value = it.label;
        save();
      });
      list.append($("div", "menu-row", iconSel, label, action, target, controls));
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
    box.append($("div", "buttons-card", head, widgetNotch(id)(ctx), itemsEditor(id, false)(ctx)));
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

const widgetNotchEditors = Object.fromEntries(WIDGETS.map((w) => [`notch:${w.id}`, widgetNotch(w.id)]));

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

export const editors: Record<string, Editor> = {
  presets,
  surfaceManager,
  buttonsWidgets,
  scopeSelector,
  monitorToggles,
  placement,
  monitorNotches,
  clockFormat,
  weatherLocation,
  tempsSensors,
  menuItems,
  ...widgetNotchEditors,
};
