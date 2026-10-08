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
// Config model: [layout] has left / center / right lists (a side for every shown
// widget) and `notch` (which of them show as their own notch). [[monitors]] rules
// may carry their own `layout`; `custom_layout` / `follow_unchanged` are UI flags.

export const WIDGETS: { id: string; label: string; icon: number }[] = [
  { id: "clock", label: "Clock", icon: 0xe823 },
  { id: "temps", label: "PC temps", icon: 0xe9ca },
  { id: "weather", label: "Weather", icon: 0xe706 },
  { id: "media", label: "Media", icon: 0xe8d6 },
  { id: "tray", label: "Tray", icon: 0xe70d },
  { id: "language", label: "Language", icon: 0xe765 },
  { id: "menu", label: "Menu", icon: 0xe700 },
];

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
const normalize = (l: Partial<LayoutCfg> | undefined): LayoutCfg => ({ ...emptyLayout(), ...structuredClone(l ?? {}) });
const sameLayout = (a: LayoutCfg, b: LayoutCfg) =>
  SIDES.every((s) => JSON.stringify(a[s]) === JSON.stringify(b[s])) &&
  JSON.stringify([...a.notch].sort()) === JSON.stringify([...b.notch].sort());
const sideOf = (l: LayoutCfg, id: string): Side | null => SIDES.find((s) => l[s].includes(id)) ?? null;

export const globalLayout = (cfg: Cfg): LayoutCfg => normalize(cfg.layout);

/** Which part of the Widgets page is being edited. */
export const scope: { monitor: string | null } = { monitor: null };
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

/** The layout a monitor actually shows. */
function monitorLayout(cfg: Cfg, name: string): LayoutCfg {
  const m = monitorsCache?.find((x) => x.name === name);
  const r = m ? ruleFor(cfg, m) : ownRule(cfg, name)?.[1];
  return r?.layout ? normalize(r.layout) : globalLayout(cfg);
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
    if (current?.layout) fresh.layout = structuredClone(current.layout);
    const star = rules.findIndex((r) => String(r.match ?? "*").trim() === "*");
    i = star < 0 ? rules.length : star;
    rules.splice(i, 0, fresh);
  }
  update(rules[i]);
  for (const r of rules) {
    delete r.notches; // legacy fields
    delete r.bar;
  }
  return [{ path: ["monitors"], value: rules }];
}

/** Edits that store `next` as the layout of the current scope. */
function layoutOps(cfg: Cfg, next: LayoutCfg): Op[] {
  if (!scope.monitor) return [{ path: ["layout"], value: next }];
  const global = globalLayout(cfg);
  return ruleOps(cfg, scope.monitor, (r) => {
    r.custom_layout = true;
    // "Follow global while unchanged": a layout equal to the global one isn't stored.
    if (r.follow_unchanged !== false && sameLayout(next, global)) delete r.layout;
    else r.layout = next;
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
  return $("div", "", bar, $("p", "card-help", help));
};

/** Per-monitor: StatusNotch on/off, custom layout, follow global while unchanged. */
const monitorToggles: Editor = (ctx) => {
  const name = scope.monitor!;
  const r = ownRule(ctx.cfg, name)?.[1];
  const m = monitorsCache?.find((x) => x.name === name);
  const effective = (m && ruleFor(ctx.cfg, m)) ?? r;
  const set = (update: (r: any) => void) => ctx.saveMany(ruleOps(ctx.cfg, name, update));
  const custom = !!effective?.layout || r?.custom_layout === true;
  const box = $("div", "stack-tight");
  box.append(
    row("StatusNotch on this monitor", undefined, sw(effective?.enabled !== false, (on) => set((x) => (x.enabled = on)))),
    row(
      "Custom layout",
      "Off: this monitor follows the global layout.",
      sw(custom, (on) =>
        set((x) => {
          x.custom_layout = on;
          if (!on) delete x.layout;
        }),
      ),
    ),
  );
  if (custom) {
    box.append(
      row(
        "Follow global while unchanged",
        "Until you change this monitor's layout, global changes still apply to it.",
        sw(r?.follow_unchanged !== false, (on) => set((x) => (x.follow_unchanged = on))),
      ),
    );
  }
  return box;
};

/** Drag widgets between Left / Center / Right / Disabled. */
const placement: Editor = (ctx) => {
  const L = currentLayout(ctx.cfg);
  const root = $("div", "sorter");
  let dragging: string | null = null;
  const columns: [Side | "disabled", string][] = [
    ["left", "Left"],
    ["center", "Center"],
    ["right", "Right"],
    ["disabled", "Disabled"],
  ];
  const isOff = (id: string) => !enabled(ctx.cfg, id) || !sideOf(L, id);

  for (const [bucket, title] of columns) {
    const ids =
      bucket === "disabled" ? WIDGETS.map((w) => w.id).filter(isOff) : L[bucket].filter((id) => enabled(ctx.cfg, id) && WIDGETS.some((w) => w.id === id));
    const list = $("div", "bucket-list");
    for (const id of ids) {
      const w = WIDGETS.find((x) => x.id === id)!;
      const chip = $("div", "chip", glyph(w.icon), w.label);
      if (bucket !== "disabled" && L.notch.includes(id)) chip.append($("span", "chip-badge", "notch"));
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
    const col = $("div", `bucket${bucket === "disabled" ? " disabled" : ""}`, $("div", "bucket-title", title), list);
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
      chips.forEach((c, j) => c.classList.toggle("drop-before", j === i && bucket !== "disabled"));
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
      const { i, chips } = indexAt(e.clientY);
      const index = chips.slice(0, i).filter((c) => c.dataset.id !== id).length;
      const next = structuredClone(L);
      for (const s of SIDES) next[s] = next[s].filter((x) => x !== id);
      const ops: Op[] = [];
      if (bucket === "disabled") {
        if (scope.monitor) {
          next.notch = next.notch.filter((x) => x !== id); // just not on this monitor
        } else {
          // Off everywhere, but keep its spot so switching it back on restores it.
          ops.push({ path: ["widgets", id, "enabled"], value: false });
          ctx.saveMany(ops);
          return;
        }
      } else {
        next[bucket].splice(Math.min(index, next[bucket].length), 0, id);
        if (!enabled(ctx.cfg, id)) ops.push({ path: ["widgets", id, "enabled"], value: true });
      }
      ctx.saveMany([...ops, ...layoutOps(ctx.cfg, next)]);
    });
    root.append(col);
  }
  const help = scope.monitor
    ? "Drag widgets between sides; Disabled hides a widget on this monitor only."
    : "Drag widgets between sides. Disabled switches a widget off everywhere. Widgets marked “notch” show as their own notch on their side.";
  return $("div", "", root, $("p", "card-help", help));
};

/** Per-monitor: which widgets show as their own notch here. */
const monitorNotches: Editor = (ctx) => {
  const L = currentLayout(ctx.cfg);
  const box = $("div", "stack-tight");
  for (const w of WIDGETS) {
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
    const L = globalLayout(ctx.cfg);
    const side = sideOf(L, widget);
    const has = L.notch.includes(widget);
    return row(
      "Own notch",
      side
        ? `Shows in the ${side} notch while the bar is closed. Notch position and corner settings are under Notches.`
        : "Place the widget on a side first (Placement above).",
      sw(has, (on) => {
        const next = structuredClone(L);
        next.notch = on ? [...next.notch, widget] : next.notch.filter((x) => x !== widget);
        ctx.saveMany([{ path: ["layout"], value: next }]);
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
  const L = globalLayout(cfg);
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
  if (cfg.layout) return [];
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

const menuItems: Editor = (ctx) => {
  const items: any[] = structuredClone(ctx.cfg.widgets?.menu?.items ?? []);
  const save = () => ctx.save(["widgets", "menu", "items"], items);
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
  return $("div", "stack-tight", list, buttons, $("p", "card-help", "Restart, shut down and sign out ask for a second click."));
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
