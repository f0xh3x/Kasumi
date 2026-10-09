import "@fontsource/inter/600.css";
import "./styles.css";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { initData } from "./data";
import { computeFrame, type Bump } from "./geometry";
import { Spring } from "./spring";
import type { Layout, NotchLayout, Rect } from "./types";
import { createPanel, createWidget, panelFor, widgetType, type Widget } from "./widgets";
import { launcherSetOpen } from "./widgets/launcher";

const svg = document.getElementById("shape") as unknown as SVGSVGElement;
const fill = document.getElementById("fill") as unknown as SVGPathElement;
const cornersPath = document.getElementById("corners") as unknown as SVGPathElement;
const content = document.getElementById("content") as HTMLDivElement;

const clamp01 = (v: number) => Math.min(1, Math.max(0, v));
const lerp = (a: number, b: number, k: number) => a + (b - a) * k;

/** "#rrggbb" + opacity → "rgba(r, g, b, a)" (other color syntaxes pass through). */
function withAlpha(color: string, alpha: number): string {
  const m = /^#([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i.exec(color.trim());
  if (!m) return color;
  const [r, g, b] = m.slice(1).map((h) => parseInt(h, 16));
  return `rgba(${r}, ${g}, ${b}, ${alpha})`;
}

function applyTheme(t: Layout["theme"]): void {
  const root = document.documentElement.style;
  root.setProperty("--bg", t.background);
  root.setProperty("--fg", t.text);
  root.setProperty("--fg-dim", withAlpha(t.text, t.dimOpacity));
  root.setProperty("--accent", t.accent);
  root.setProperty("--warn", t.warn);
  root.setProperty("--shape-opacity", String(Math.min(1, Math.max(0, (t.opacity ?? 100) / 100))));
  document.body.style.fontSize = `${t.fontSize}px`;
}

function div(className: string): HTMLDivElement {
  const d = document.createElement("div");
  d.className = className;
  return d;
}

/**
 * Something that hangs from the bar's bottom edge: a notch (always present,
 * optionally with a panel), or a bar widget's dropdown (only while open).
 */
interface Drop {
  key: string;
  x: number;
  width: number;
  height: number;
  inHull: boolean;
  panelWidth: number;
  panelHeight: number;
  hasPanel: boolean;
  spring: Spring;
  /** Notches: animated left edge, so position changes (docking) slide. */
  xs: Spring | null;
  /** Notches: 1 shown, 0 retracted into the top edge (hidden under a window). */
  shown?: Spring;
  /** Notches: their layout (dock morph between rest and open). */
  notch?: NotchLayout;
  /** Dock notches: a panel per widget that has one; `panelEl` is the open one. */
  panels?: Map<string, HTMLDivElement>;
  activePanel?: string;
  /** Notch content, laid out in the notch's row. */
  slot: HTMLDivElement | null;
  panelEl: HTMLDivElement | null;
  /** For bar dropdowns: the widget the panel drops from. */
  anchor: HTMLElement | null;
}

/** Space between a notch's content and the start of its curves, each side. */
const NOTCH_PAD = 10;

class Overlay {
  private layout: Layout | null = null;
  private expanded = false;
  private expand = new Spring(0, 0, 420, 32);
  private spread = new Spring(0, 0, 260, 28);
  private drops: Drop[] = [];

  // OSD: a notch briefly grows into a pill with an icon, level bar and value.
  private osd = new Spring(0, 0, 420, 32);
  private osdTimer = 0;
  private osdEl = div("osd");
  private osdIcon = document.createElement("span");
  private osdFill = div("osd-fill");
  private osdValue = document.createElement("span");

  // Keyboard-layout peek: a small notch pops out at the language widget.
  private peek = new Spring(0, 0, 420, 32);
  private peekTimer = 0;
  private peekDrop: Drop | null = null;
  private peekEl = div("peek");
  private langEl: HTMLElement | null = null;

  // Notch widgets glide to their place in the bar as it opens, then hand over to
  // the bar's own copy (same spot, so the swap is invisible).
  private barEls = new Map<string, HTMLElement>();
  private notchEls: { slot: HTMLDivElement; name: string; el: HTMLElement }[] = [];

  private widgets: Widget[] = [];
  /** Opens the launcher's panel on this overlay (its widget's, or a fallback one). */
  private openLauncher: (() => void) | null = null;
  private bar = div("bar");
  private groups: Partial<Record<"left" | "center" | "right", HTMLDivElement>> = {};
  /** Center widgets around a middle notch: [before it, after it]. */
  private centerSplit: [HTMLDivElement, HTMLDivElement] | null = null;
  // Notch widths follow their content: measured here, positioned by Rust.
  private notchObserver = new ResizeObserver(() => this.scheduleMeasure());
  // And now and then regardless: a missed resize would leave a notch the wrong
  // width (content spilling out of it). Cheap: only sends when a width changed.
  readonly remeasure = window.setInterval(() => this.checkWidths(), 2000);
  private measureTimer = 0;
  private lastWidths = "";

  private running = false;
  private lastTime = 0;
  private lastRegions = "";
  private requestedHeight = 0;

  apply(layout: Layout): void {
    const first = this.layout === null;
    const prevSprings = new Map(this.drops.map((d) => [d.key, d.spring]));
    const prevX = new Map(this.drops.flatMap((d) => (d.xs ? [[d.key, d.xs] as const] : [])));
    this.layout = layout;
    applyTheme(layout.theme);
    tooltips = layout.tooltips !== false;
    if (!tooltips) stripTitles(document.body);
    const html = document.documentElement;
    html.classList.toggle("edge-bottom", layout.edge === "bottom");
    html.classList.toggle("mode-dock", layout.mode === "dock");
    html.className = html.className.replace(/\b(rest|open)-\w+/g, "").trim();
    if (layout.mode === "dock") html.classList.add(`rest-${layout.rest}`, `open-${layout.open}`);

    const a = layout.animation;
    this.expand.configure(a.expandStiffness, a.expandDamping, a.mass);
    this.spread.configure(a.spreadStiffness, a.spreadDamping, a.mass);
    this.osd.configure(a.expandStiffness, a.expandDamping, a.mass);
    this.peek.configure(a.expandStiffness, a.expandDamping, a.mass);

    if (first) {
      this.expanded = layout.expanded;
      document.documentElement.classList.toggle("bar-open", layout.expanded);
      this.expand.target = layout.expanded ? 1 : 0;
      this.expand.snap();
    }
    this.spread.target = this.spreadTarget();
    if (first || layout.animation.style !== "spread") this.spread.snap();

    // Oversized so the window can grow (dropdowns) without resizing the SVG.
    svg.setAttribute("width", String(layout.width));
    svg.setAttribute("height", "2000");
    svg.setAttribute("viewBox", `0 0 ${layout.width} 2000`);
    this.requestedHeight = 0;
    this.buildDom(layout, prevSprings, prevX);
    this.layoutBar();
    this.notchObserver.disconnect();
    for (const d of this.drops) for (const c of d.slot?.children ?? []) this.notchObserver.observe(c);
    this.scheduleMeasure();
    this.measureBarDrops();
    this.render();
    this.kick();
    this.sendRegions();
  }

  setExpanded(expanded: boolean): void {
    this.expanded = expanded;
    document.documentElement.classList.toggle("bar-open", expanded);
    this.layoutBar();
    this.expand.target = expanded ? 1 : 0;
    this.spread.target = this.spreadTarget();
    if (!expanded) for (const d of this.drops) d.spring.target = 0;
    this.kick();
    this.sendRegions();
  }

  /** Rust moved notches (e.g. a docking notch left its corner): slide them over. */
  updatePositions(notches: NotchLayout[]): void {
    const L = this.layout;
    if (!L) return;
    for (const n of notches) {
      const i = L.notches.findIndex((m) => m.id === n.id);
      if (i >= 0) L.notches[i] = n;
      const d = this.drops.find((d) => d.key === `notch:${n.id}`);
      if (d?.xs) {
        d.x = n.x;
        d.xs.target = n.x;
        d.width = n.width;
        if (d.slot) {
          d.slot.style.width = `${n.width}px`;
          this.padEdge(d.slot, n);
        }
      }
      if (d?.shown) d.shown.target = n.hidden ? 0 : 1;
      if (d) d.notch = n;
      this.markHidden(n);
    }
    this.layoutBar();
    this.kick();
    this.sendRegions();
  }

  /**
   * Bars: notch widgets stay where they are when the bar opens, so the bar's own
   * widgets make room: a side's group starts beside its notch (on the inner side),
   * center widgets sit before / after a middle notch.
   */
  private layoutBar(): void {
    const L = this.layout;
    if (!L || L.mode === "dock") return;
    // A notch's content edge: past its flare and padding.
    const inset = L.geometry.notchCornerRadius + NOTCH_PAD;
    const gap = L.bar.gap;
    const pad = L.bar.padding;
    const sideOf = (n: NotchLayout) => (["left", "center", "right"] as const).find((s) => n.widgets.some((w) => L.bar[s].includes(w)));
    let leftEnd = 0;
    let rightStart = L.width;
    let middle: NotchLayout | null = null;
    for (const n of L.notches) {
      if (n.hidden || !n.widgets.length) continue;
      const side = sideOf(n);
      if (side === "left") leftEnd = Math.max(leftEnd, n.x + n.width - inset);
      else if (side === "right") rightStart = Math.min(rightStart, n.x + inset);
      else if (side === "center") middle = n;
    }
    if (this.groups.left) this.groups.left.style.marginLeft = leftEnd ? `${Math.max(0, leftEnd + gap - pad)}px` : "";
    if (this.groups.right) this.groups.right.style.marginRight = rightStart < L.width ? `${Math.max(0, L.width - rightStart + gap - pad)}px` : "";
    if (this.centerSplit) {
      const [before, after] = this.centerSplit;
      const x0 = middle ? middle.x + inset : L.width / 2;
      const x1 = middle ? middle.x + middle.width - inset : L.width / 2;
      before.style.right = `${L.width - x0 + gap}px`;
      after.style.left = `${x1 + gap}px`;
    }
  }

  /** "drop" keeps the bar full width so it slides out of the top edge; "spread" grows it sideways. */
  private spreadTarget(): number {
    return this.layout?.animation.style === "spread" && !this.expanded ? 0 : 1;
  }

  private toggleDrop(drop: Drop, force?: boolean): void {
    if (!this.expanded || !drop.hasPanel) return;
    const open = force ?? drop.spring.target < 0.5;
    if (open) this.measureBarDrops();
    for (const d of this.drops) d.spring.target = d === drop && open ? 1 : 0;
    this.kick();
    this.sendRegions();
  }

  /** Rust opened (pinned + focused this overlay) or closed the launcher. */
  setLauncher(open: boolean): void {
    if (open) this.openLauncher?.();
    else this.closeAll();
    launcherSetOpen(open);
  }

  /** Dock: a notch widget's panel morphs out of its notch (one open at a time). */
  private toggleDockPanel(drop: Drop, name: string, force?: boolean): void {
    if (!this.expanded || !drop.panels) return;
    const open = force ?? !(drop.activePanel === name && drop.spring.target > 0.5);
    for (const d of this.drops) if (d !== drop) d.spring.target = 0;
    if (open) {
      for (const [n, el] of drop.panels) if (n !== name) el.style.visibility = "hidden";
      drop.activePanel = name;
      drop.panelEl = drop.panels.get(name) ?? null;
      drop.hasPanel = true;
      this.measureBarDrops();
    }
    drop.spring.target = open ? 1 : 0;
    this.kick();
    this.sendRegions();
  }

  /** A dropdown's content changed size: re-measure and reshape if it's open. */
  private resizeDrops(): void {
    if (!this.layout) return;
    this.measureBarDrops();
    this.kick();
    this.sendRegions();
  }

  private closeAll = (): void => {
    for (const d of this.drops) d.spring.target = 0;
    this.kick();
    this.sendRegions();
  };

  private buildDom(L: Layout, prevSprings: Map<string, Spring>, prevX: Map<string, Spring>): void {
    for (const w of this.widgets) w.dispose?.();
    this.widgets = [];
    this.drops = [];
    this.langEl = null;
    this.barEls.clear();
    this.notchEls = [];
    this.openLauncher = null;
    this.lastWidths = "";
    content.replaceChildren();

    const a = L.animation;
    const spring = (key: string) => {
      const sp = prevSprings.get(key) ?? new Spring(0, 0, a.panelStiffness, a.panelDamping, a.mass);
      sp.configure(a.panelStiffness, a.panelDamping, a.mass);
      return sp;
    };
    const ctx = { close: this.closeAll, resize: () => this.resizeDrops() };
    const opts = (name: string): Record<string, unknown> => ({ ...(L.widgets[name] ?? {}), _device: L.device });
    const mountPanel = (name: string, width?: number, height?: number, extra: Record<string, unknown> = {}) => {
      const panel = div(`panel panel-${widgetType(name)}`);
      if (width) panel.style.width = `${width}px`;
      if (height) panel.style.height = `${height}px`;
      const w = createPanel(name, { ...opts(name), ...extra }, ctx);
      this.widgets.push(w);
      panel.append(w.el);
      content.append(panel);
      return panel;
    };

    const dock = L.mode === "dock";
    // Full-width bar row: slides down with the hull (docks have none).
    this.bar = div("bar");
    if (dock) this.bar.style.display = "none";
    this.bar.style.width = `${L.width}px`;
    this.bar.style.height = `${L.geometry.expandedHeight}px`;
    this.bar.style.padding = `0 ${L.bar.padding}px`;
    content.append(this.bar);
    // A widget with a panel of the same name gets a dropdown under it in the open bar.
    const dropdown = (name: string, w: Widget) => {
      const panelName = panelFor(name);
      if (widgetType(name) === "launcher") this.openLauncher = () => w.el.dispatchEvent(new CustomEvent("notch:panel"));
      if (!panelName) return;
      const drop: Drop = {
        key: `bar:${name}`,
        x: 0,
        width: 0,
        height: 0,
        inHull: false,
        panelWidth: 0,
        panelHeight: 0,
        hasPanel: true,
        spring: spring(`bar:${name}`),
        xs: null,
        slot: null,
        panelEl: mountPanel(panelName, Number(opts(name).width) || undefined),
        anchor: w.el,
      };
      if (w.panelTrigger !== "event") {
        w.el.classList.add("clickable");
        w.el.addEventListener("click", () => this.toggleDrop(drop));
      }
      w.el.addEventListener("notch:panel", () => this.toggleDrop(drop, true));
      this.drops.push(drop);
    };
    // Bars: a notch widget stays in its notch when the bar opens (one copy, no
    // hand-over); the bar row holds the rest and makes room for it (layoutBar).
    const notched = new Set(dock ? [] : L.notches.flatMap((n) => n.widgets));
    const addGroup = (cls: string, names: string[]) => {
      const group = div(`group ${cls}`);
      group.style.gap = `${L.bar.gap}px`;
      this.bar.append(group);
      for (const name of names) {
        const w = createWidget(name, opts(name));
        this.widgets.push(w);
        group.append(w.el);
        this.barEls.set(name, w.el);
        if (name === "language") this.langEl = w.el;
        dropdown(name, w);
      }
      return group;
    };
    this.centerSplit = null;
    for (const side of ["left", "center", "right"] as const) {
      const names = L.bar[side].filter((n) => !notched.has(n));
      const first = L.bar[side].findIndex((n) => notched.has(n));
      if (side === "center" && first >= 0) {
        // Around a middle notch: widgets listed before its widgets sit left of it, the rest right.
        const before = L.bar.center.slice(0, first).filter((n) => !notched.has(n));
        const after = L.bar.center.slice(first).filter((n) => !notched.has(n));
        // Not ".center": that pins it to the middle grid column, which would then be
        // its containing block (left/right would count from the column, not the bar).
        this.centerSplit = [addGroup("split before", before), addGroup("split after", after)];
        this.groups.center = undefined;
        continue;
      }
      this.groups[side] = addGroup(side, names);
    }

    // Notches, with their optional panels.
    for (const n of L.notches) {
      const slot = div("slot");
      slot.dataset.id = n.id;
      slot.style.width = `${n.width}px`;
      // Same spacing as the bar, so widgets don't shift as it opens.
      slot.style.gap = `${L.bar.gap}px`;
      this.padEdge(slot, n);
      slot.dataset.notch = n.id;
      const panels = new Map<string, HTMLDivElement>();
      for (const name of n.widgets) {
        // Bar notches show a compact copy (the bar has the full one); a dock's
        // notches are the widgets themselves.
        const w = createWidget(name, dock ? opts(name) : { ...opts(name), inNotch: true });
        this.widgets.push(w);
        slot.append(w.el);
        this.notchEls.push({ slot, name, el: w.el });
        if (name === "language") this.langEl = w.el;
        if (!dock) {
          dropdown(name, w);
          if (panelFor(name) || widgetType(name) === "launcher") slot.classList.add("clickable");
          continue;
        }
        if (widgetType(name) === "launcher") this.openLauncher = () => w.el.dispatchEvent(new CustomEvent("notch:panel"));
        const panelName = panelFor(name);
        if (panelName) {
          const panel = mountPanel(panelName, Number(opts(name).width) || undefined);
          panel.style.visibility = "hidden";
          panels.set(name, panel);
          if (w.panelTrigger !== "event") {
            w.el.classList.add("clickable");
            w.el.addEventListener("click", () => this.toggleDockPanel(drop, name));
          }
          w.el.addEventListener("notch:panel", () => this.toggleDockPanel(drop, name, true));
        }
      }
      this.markHidden(n);
      content.append(slot);
      const drop: Drop = {
        notch: n,
        panels,
        key: `notch:${n.id}`,
        x: n.x,
        width: n.width,
        height: n.height,
        inHull: true,
        panelWidth: n.panel ? n.panelWidth : n.width,
        panelHeight: n.panel ? n.panelHeight : 0,
        hasPanel: !!n.panel,
        spring: spring(`notch:${n.id}`),
        xs: (() => {
          const xs = prevX.get(`notch:${n.id}`) ?? new Spring(n.x, n.x, a.spreadStiffness, a.spreadDamping, a.mass);
          xs.configure(a.spreadStiffness, a.spreadDamping, a.mass);
          xs.target = n.x;
          return xs;
        })(),
        shown: new Spring(n.hidden ? 0 : 1, n.hidden ? 0 : 1, a.spreadStiffness, a.spreadDamping, a.mass),
        slot,
        panelEl: n.panel ? mountPanel(n.panel, n.panelWidth || undefined, n.panelHeight || undefined) : null,
        anchor: null,
      };
      if (n.panel) {
        slot.classList.add("clickable");
        slot.addEventListener("click", () => this.toggleDrop(drop));
      }
      this.drops.push(drop);
    }

    // No launcher widget here: the hotkey opens a panel with its own search field,
    // from the middle notch of a dock, or the middle of an open bar.
    if (L.launcher && !this.openLauncher) {
      if (dock) {
        const middle = this.drops
          .filter((d) => d.inHull && d.panels)
          .sort((a, b) => Math.abs(a.x + a.width / 2 - L.width / 2) - Math.abs(b.x + b.width / 2 - L.width / 2))[0];
        if (middle?.panels) {
          const panel = mountPanel("launcher", undefined, undefined, { _ownField: true });
          panel.style.visibility = "hidden";
          middle.panels.set("launcher", panel);
          this.openLauncher = () => this.toggleDockPanel(middle, "launcher", true);
        }
      } else {
        const anchor = div("launcher-anchor");
        this.bar.append(anchor);
        const drop: Drop = {
          key: "bar:launcher",
          x: 0,
          width: 0,
          height: 0,
          inHull: false,
          panelWidth: 0,
          panelHeight: 0,
          hasPanel: true,
          spring: spring("bar:launcher"),
          xs: null,
          slot: null,
          panelEl: mountPanel("launcher", undefined, undefined, { _ownField: true }),
          anchor,
        };
        this.drops.push(drop);
        this.openLauncher = () => this.toggleDrop(drop, true);
      }
    }

    // The keyboard peek: a zero-size bump until it pops (x holds its center).
    this.peekDrop = {
      key: "peek",
      x: 0,
      width: 0,
      height: 0,
      inHull: false,
      panelWidth: 0,
      panelHeight: 0,
      hasPanel: false,
      spring: new Spring(0, 0, 1, 1),
      xs: null,
      slot: null,
      panelEl: null,
      anchor: null,
    };
    this.drops.push(this.peekDrop);
    this.peekEl = div("peek");
    content.append(this.peekEl);

    // OSD content, drawn over the OSD notch (clipped by the shape like everything else).
    this.osdIcon.className = "icon";
    this.osdValue.className = "osd-value";
    const track = div("osd-track");
    track.append(this.osdFill);
    this.osdEl = div("osd");
    this.osdEl.append(this.osdIcon, track, this.osdValue);
    content.append(this.osdEl);
  }

  /** At most every 80 ms (a widget that keeps resizing mustn't postpone it forever). */
  private scheduleMeasure(): void {
    if (this.measureTimer) return;
    this.measureTimer = window.setTimeout(() => {
      this.measureTimer = 0;
      this.measureNotches();
    }, 80);
  }

  /** A notch slot's content width as laid out now: full, and without the widgets
   *  that hide under windows. `pad`: its curves and padding. */
  private slotWidths(n: NotchLayout, slot: HTMLElement, pad: number): [number, number] {
    const L = this.layout!;
    const hides = (name: string) => L.widgets[name]?.hide_when_maximized === true;
    const gap = parseFloat(getComputedStyle(slot).columnGap) || 0;
    const total = pad + this.edgeExtra(n);
    const span = (els: HTMLElement[]) =>
      els.length ? Math.ceil(els.reduce((sum, k) => sum + k.offsetWidth, 0) + gap * (els.length - 1) + total) : 0;
    // Widgets with nothing to show (e.g. media while idle) take no room.
    const kids = this.notchEls.filter((e) => e.slot === slot && e.el.offsetWidth > 0);
    return [span(kids.map((k) => k.el)), span(kids.filter((k) => !hides(k.name)).map((k) => k.el))];
  }

  /** Cheap check (reads only, nothing restyled): do the notches still fit their
   *  content as last reported? A missed resize would otherwise leave a notch the
   *  wrong width, its content spilling out. */
  private checkWidths(): void {
    const L = this.layout;
    if (!L || !this.lastWidths || this.running) return;
    const last = JSON.parse(this.lastWidths) as Record<string, { full: number; open: number }>;
    const dock = L.mode === "dock";
    const open = dock && this.expanded;
    const h = open ? L.openHeight : L.restHeight;
    const pad = 2 * L.geometry.notchCornerRadius * (dock ? h / L.geometry.notchHeight : 1) + 2 * NOTCH_PAD;
    for (const n of L.notches) {
      const slot = this.drops.find((d) => d.key === `notch:${n.id}`)?.slot;
      if (!slot) continue;
      const [now] = this.slotWidths(n, slot, pad);
      const was = last[n.id] ? (open ? last[n.id].open : last[n.id].full) : 0;
      if (Math.abs(now - was) > 1) {
        this.scheduleMeasure();
        return;
      }
    }
  }

  /** Reports each notch's content width (plus its curves and padding) to Rust. */
  private measureNotches(): void {
    const L = this.layout;
    if (!L) return;
    const dock = L.mode === "dock";
    const base = (h: number) => 2 * L.geometry.notchCornerRadius * (dock ? h / L.geometry.notchHeight : 1) + 2 * NOTCH_PAD;
    const widths: Record<string, { full: number; compact: number; open: number }> = {};
    // Docks: measure the content at the rest and the open size (widgets size with --h).
    const root = document.documentElement.style;
    const prevH = root.getPropertyValue("--h");
    const prevT = root.getPropertyValue("--t");
    const at = (h: number, t: number, read: () => number[]) => {
      if (!dock) return read();
      root.setProperty("--h", `${h}px`);
      root.setProperty("--t", String(t));
      const v = read();
      root.setProperty("--h", prevH);
      root.setProperty("--t", prevT);
      return v;
    };
    for (const n of L.notches) {
      const slot = this.drops.find((d) => d.key === `notch:${n.id}`)?.slot;
      if (!slot) continue;
      const measure = (h: number, t: number) => at(h, t, () => this.slotWidths(n, slot, base(h)));
      const [full, compact] = measure(L.restHeight, 0);
      const [open] = dock ? measure(L.openHeight, 1) : [full];
      if (full || open) widths[n.id] = { full, compact, open };
    }
    const sent = JSON.stringify(widths);
    if (sent === "{}" || sent === this.lastWidths) return;
    this.lastWidths = sent;
    invoke("set_notch_widths", { widths }).catch((e) => report(`set_notch_widths: ${e}`));
  }

  /** Extra room on a corner-hugging notch's outer side, up to the bar's edge padding. */
  private edgeExtra(n: NotchLayout): number {
    // Only bars: there the corner notch's content lines up with the open bar's.
    return n.edge && this.layout!.mode === "bar" ? Math.max(0, this.layout!.bar.padding - NOTCH_PAD) : 0;
  }

  /** Content of a corner notch sits as far from the screen edge as in the open bar. */
  private padEdge(slot: HTMLDivElement, n: NotchLayout): void {
    const extra = `${this.edgeExtra(n)}px`;
    slot.style.paddingLeft = n.edge === "left" ? extra : "";
    slot.style.paddingRight = n.edge === "right" ? extra : "";
  }

  /** Widgets hidden under a window leave the notch's flow (still measurable) and fade. */
  private markHidden(n: NotchLayout): void {
    for (const e of this.notchEls) {
      if (e.slot.dataset.notch === n.id) e.el.classList.toggle("covered", n.hiddenWidgets.includes(e.name));
    }
  }

  /** Bar dropdowns hang from their widget; their size is the panel's natural size. */
  private measureBarDrops(): void {
    const L = this.layout;
    if (!L) return;
    let tallest = 0;
    for (const d of this.drops) {
      if (!d.panelEl) continue;
      if (!d.anchor) {
        // Notch panels fit their content's height (panel_height is only the
        // starting value), and its width too unless one is configured.
        const notch = this.layout?.notches.find((n) => `notch:${n.id}` === d.key);
        if (!notch?.panelWidth) {
          d.panelEl.style.width = "auto";
          d.panelWidth = d.panelEl.offsetWidth;
          d.panelEl.style.width = `${d.panelWidth}px`;
        }
        // A dock notch's panel is at least as wide as the open notch, so the
        // morphed shape never clips the notch's own row; its content is centered.
        const rowWidth = L.mode === "dock" && d.notch ? d.notch.openWidth : 0;
        d.panelEl.classList.toggle("widened", d.panelWidth < rowWidth);
        if (d.panelWidth < rowWidth) {
          d.panelWidth = rowWidth;
          d.panelEl.style.width = `${rowWidth}px`;
        }
        d.panelEl.style.height = "auto";
        d.panelHeight = d.panelEl.offsetHeight;
        d.panelEl.style.height = `${d.panelHeight}px`;
        tallest = Math.max(tallest, d.panelHeight);
        continue;
      }
      d.panelWidth = d.panelEl.offsetWidth;
      d.panelHeight = d.panelEl.offsetHeight;
      // Dropdowns have zero collapsed width, so x is the panel's center.
      const anchorBox = d.anchor.getBoundingClientRect();
      const panelBox = d.panelEl.getBoundingClientRect();
      const panelIcon = d.panelEl.querySelector('[data-align="icon"] .icon')?.getBoundingClientRect();
      if (panelIcon) {
        // Panels that opt in (the menu) line their icon column up under the widget's icon.
        const anchorIcon = (d.anchor.querySelector(".icon") ?? d.anchor).getBoundingClientRect();
        d.x = anchorIcon.left - (panelIcon.left - panelBox.left) + d.panelWidth / 2;
      } else {
        // Everything else centers under the whole widget.
        d.x = anchorBox.left + anchorBox.width / 2;
      }
      tallest = Math.max(tallest, d.panelHeight);
    }
    // Rust sizes the window for notch panels only; ask for room for dropdowns too.
    const needed = (L.mode === "dock" ? L.openHeight : L.geometry.expandedHeight) + tallest;
    if (needed > this.requestedHeight) {
      this.requestedHeight = needed;
      invoke("set_content_height", { height: needed }).catch((e) => report(`set_content_height: ${e}`));
    }
  }

  /** Drops in path order (left to right by collapsed center). */
  private ordered(): Drop[] {
    return [...this.drops].sort((a, b) => a.x + a.width / 2 - (b.x + b.width / 2));
  }

  /** The notch that grows into the OSD pill. */
  private osdDrop(): Drop | undefined {
    const L = this.layout;
    if (!L) return undefined;
    const holder = L.notches.find((n) => n.id === L.osd.notch || n.widgets.includes(L.osd.notch));
    return (
      (holder && this.drops.find((d) => d.key === `notch:${holder.id}`)) ??
      this.drops.find((d) => d.key === "notch:center") ??
      this.drops.find((d) => d.inHull)
    );
  }

  /** Shows a level on the OSD notch, then shrinks back after the configured duration. */
  showOsd(e: { kind: "volume" | "brightness" | "keyboard"; level: number; muted: boolean; text?: string }): void {
    const L = this.layout;
    if (!L || !this.osdDrop()) return;
    const pct = Math.round(e.level * 100);
    let glyph = 0xe706; // Brightness
    if (e.kind === "keyboard") glyph = 0xe765;
    if (e.kind === "volume") {
      glyph = e.muted ? 0xe74f : pct === 0 ? 0xe992 : pct < 34 ? 0xe993 : pct < 67 ? 0xe994 : 0xe995;
    }
    this.osdIcon.textContent = String.fromCharCode(glyph);
    this.osdFill.style.width = `${pct}%`;
    this.osdEl.classList.toggle("muted", e.muted);
    this.osdValue.textContent = e.text ?? String(pct);
    this.osdEl.classList.toggle("text", e.text !== undefined);
    this.osd.target = 1;
    clearTimeout(this.osdTimer);
    this.osdTimer = window.setTimeout(() => {
      this.osd.target = 0;
      this.kick();
    }, L.osd.durationMs);
    this.kick();
  }

  /** Keyboard layout changed: pop a small notch at the language widget (or use the OSD). */
  showKeyboard(e: { code: string; name: string }): void {
    const L = this.layout;
    if (!L) return;
    // The language widget shows the new layout itself whenever it's visible.
    if (this.languageVisible()) return;
    const anchor = this.langEl?.getBoundingClientRect();
    // Docks (tiny handle / covered): grow the OSD pill rather than a second bump.
    if (!anchor || !this.peekDrop || this.expanded || L.mode === "dock") {
      this.showOsd({ kind: "keyboard", level: 0, muted: false, text: e.code });
      return;
    }
    this.peekDrop.x = anchor.left + anchor.width / 2;
    const glyph = document.createElement("span");
    glyph.className = "icon";
    glyph.textContent = String.fromCharCode(0xe765);
    const code = document.createElement("span");
    code.textContent = e.code;
    this.peekEl.replaceChildren(glyph, code);
    this.peek.target = 1;
    clearTimeout(this.peekTimer);
    this.peekTimer = window.setTimeout(() => {
      this.peek.target = 0;
      this.kick();
    }, L.osd.durationMs);
    this.kick();
  }

  /** Whether the language widget can be seen right now (open bar, or in a notch). */
  private languageVisible(): boolean {
    const L = this.layout;
    if (!L) return false;
    if (this.expanded && this.langEl) return true;
    const notch = L.notches.find((n) => n.widgets.includes("language"));
    if (!notch || notch.hidden || notch.hiddenWidgets.includes("language")) return false;
    // A dock notch at the tiny handle size shows no content.
    return L.mode !== "dock" || (!notch.shrunk && L.rest !== "tiny");
  }

  private bumps(drops: Drop[], useTarget: boolean): Bump[] {
    const L = this.layout!;
    const od = this.osdDrop();
    const o = Math.max(0, useTarget ? this.osd.target : this.osd.value);
    const k = Math.max(0, useTarget ? this.peek.target : this.peek.value);
    return drops.map((d) => {
      if (d === this.peekDrop) {
        const width = L.osd.keyboardWidth * k;
        return {
          x: d.x - width / 2,
          width,
          height: L.geometry.notchHeight * k,
          panelWidth: width,
          panelHeight: 0,
          p: 0,
          inHull: false,
          rounded: 0,
        };
      }
      let x = d.xs ? (useTarget ? d.xs.target : d.xs.value) : d.x;
      let width = d.width;
      let height = d.height;
      let scale: number | undefined;
      let panelTop: number | undefined;
      if (L.mode === "dock" && d.notch) {
        // Grow from the rest size to the open size in place.
        const e = Math.max(0, useTarget ? this.expand.target : this.expand.value);
        const n = d.notch;
        x = lerp(x, n.openX, e);
        width = lerp(n.width, n.openWidth, e);
        height = lerp(n.height, n.openHeight, e);
        scale = height / L.geometry.notchHeight;
        panelTop = height;
      }
      if (d.shown) {
        // Retracting (hidden under a window): shrink into the top edge around the center.
        const s = clamp01(useTarget ? d.shown.target : d.shown.value);
        x += (width * (1 - s)) / 2;
        width *= s;
        height *= s;
      }
      if (d === od && o > 0) {
        // Grow around the notch's center.
        const cx = x + d.width / 2;
        width = d.width + (L.osd.width - d.width) * o;
        height = d.height + (L.osd.height - d.height) * o;
        x = cx - width / 2;
      }
      return {
        x,
        width,
        height,
        panelWidth: d.panelWidth,
        panelHeight: d.panelHeight,
        p: d.hasPanel ? (useTarget ? d.spring.target : d.spring.value) : 0,
        inHull: d.inHull,
        scale,
        panelTop,
      };
    });
  }

  private render(): void {
    const L = this.layout;
    if (!L) return;
    const H = L.geometry.expandedHeight;
    const drops = this.ordered();
    const dock = L.mode === "dock";
    const bumps = this.bumps(drops, false);
    const frame = computeFrame(L, dock ? 0 : this.expand.value, dock ? 0 : this.spread.value, bumps);
    if (dock) {
      // Widgets can size themselves with these (e.g. task icons grow as the dock opens).
      const t = clamp01(this.expand.value);
      const root = document.documentElement.style;
      root.setProperty("--t", t.toFixed(3));
      root.setProperty("--h", `${lerp(L.restHeight, L.openHeight, t).toFixed(2)}px`);
    }

    queueMicrotask(() => this.glide());
    fill.setAttribute("d", frame.d);
    cornersPath.setAttribute("d", frame.corners);
    content.style.clipPath = `path("${frame.d}")`;

    this.bar.style.transform = `translateY(${frame.base - H}px)`;
    this.bar.style.opacity = String(clamp01((this.expand.value - 0.25) / 0.6));

    const od = this.osdDrop();
    const o = clamp01(this.osd.value);
    drops.forEach((d, i) => {
      const bf = frame.bumps[i];
      if (d.slot) {
        // Docks: the slot follows the notch's current (morphing) width.
        const w = dock ? bumps[i].width : d.width;
        d.slot.style.height = `${bf.rowDepth}px`;
        d.slot.style.transform = `translateX(${bf.anchorCx - w / 2}px)`;
        if (dock) d.slot.style.width = `${w}px`;
        // Cross-fade the notch's own content with the OSD; a tiny dock handle hides it.
        const shown = d.shown ? clamp01(d.shown.value * 1.5 - 0.5) : 1;
        const fits = dock ? clamp01((bf.rowDepth - 16) / 10) : 1;
        d.slot.style.opacity = String(shown * fits * (d === od ? 1 - clamp01(o * 1.8) : 1));
      }
      if (d === this.peekDrop) {
        const k = clamp01(this.peek.value);
        this.peekEl.style.transform = `translate(${bf.cx}px, ${bf.rowDepth / 2}px) translate(-50%, -50%)`;
        this.peekEl.style.opacity = String(clamp01(k * 1.8 - 0.5) * (1 - clamp01(this.expand.value)));
        this.peekEl.style.visibility = k > 0.01 ? "visible" : "hidden";
      }
      if (d === od) {
        const inner = L.osd.width - 2 * L.geometry.notchCornerRadius - 8;
        this.osdEl.style.width = `${inner}px`;
        this.osdEl.style.transform = `translate(${bf.cx - inner / 2}px, ${bf.rowDepth / 2}px) translateY(-50%)`;
        this.osdEl.style.opacity = String(clamp01(o * 1.8 - 0.5));
        this.osdEl.style.visibility = o > 0.01 ? "visible" : "hidden";
      }
      if (d.panelEl) {
        const p = d.spring.value;
        d.panelEl.style.visibility = p > 0.01 ? "visible" : "hidden";
        d.panelEl.style.opacity = String(clamp01(p * 1.4 - 0.2));
        d.panelEl.style.transform = `translate(${bf.cx - d.panelWidth / 2}px, ${bf.depth - d.panelHeight}px)`;
      }
    });
  }

  /**
   * Moves each notch widget from its spot in the notch toward its twin's spot
   * in the bar, in step with the bar opening. Once the bar is open the twin
   * takes over; while it's closing the notch copy is shown again and glides back.
   */
  private glide(): void {
    const L = this.layout;
    if (!L) return;
    const handedOver = this.expanded && this.expand.value >= 0.97;
    const k = clamp01(this.expand.value);
    for (const { slot, name, el } of this.notchEls) {
      const twin = this.barEls.get(name);
      if (!twin) continue;
      twin.style.visibility = handedOver ? "" : "hidden";
      twin.classList.toggle("arrived", handedOver);
      el.style.visibility = handedOver ? "hidden" : "";
      if (handedOver) continue;
      // Glide by the part both copies share (e.g. the media title), else the whole widget.
      const src = el.querySelector<HTMLElement>("[data-glide]") ?? el;
      const dst = twin.querySelector<HTMLElement>("[data-glide]") ?? twin;
      // offsetLeft is relative to the slot / bar (the positioned ancestors), for
      // the widget and its parts alike.
      // Natural position of the notch copy (content coords: slot is at its translateX).
      const slotX = new DOMMatrixReadOnly(getComputedStyle(slot).transform).m41;
      const nx = slotX + src.offsetLeft + src.offsetWidth / 2;
      const ny = el.offsetTop + el.offsetHeight / 2;
      // Where the twin's part sits in the fully open bar (the bar is at 0,0 when open).
      const tx = dst.offsetLeft + dst.offsetWidth / 2;
      const ty = twin.offsetTop + twin.offsetHeight / 2;
      // Bottom surfaces are drawn flipped (see styles.css): rows mirror around their centers.
      const dy = (L.edge === "bottom" ? ny - ty : ty - ny) * k;
      el.style.transform = `translate(${(tx - nx) * k}px, ${dy}px)`;
    }
  }

  private kick(): void {
    if (this.running) return;
    this.running = true;
    this.lastTime = performance.now();
    requestAnimationFrame(this.loop);
  }

  private loop = (now: number): void => {
    const dt = Math.min(0.05, (now - this.lastTime) / 1000);
    this.lastTime = now;
    const springs = [
      this.expand,
      this.spread,
      this.osd,
      this.peek,
      ...this.drops.flatMap((d) => [d.spring, ...(d.xs ? [d.xs] : []), ...(d.shown ? [d.shown] : [])]),
    ];
    for (const s of springs) s.step(dt);
    if (springs.every((s) => s.settled)) {
      for (const s of springs) s.snap();
      this.running = false;
      this.render();
      return;
    }
    this.render();
    requestAnimationFrame(this.loop);
  };

  /** Tells Rust where the overlay is interactive, for click-through and hover-leave. */
  private sendRegions(): void {
    const L = this.layout;
    if (!L) return;
    const regions: Rect[] = [];
    if (this.expanded && L.mode === "dock") {
      const drops = this.ordered();
      const bumps = this.bumps(drops, true);
      const final = computeFrame(L, 0, 0, bumps);
      drops.forEach((d, i) => {
        if (!d.inHull) return;
        const bf = final.bumps[i];
        const r = L.geometry.concaveRadius * (bumps[i].scale ?? 1);
        regions.push({ x: bf.left - r, y: 0, width: bf.right - bf.left + 2 * r, height: bf.depth });
      });
    } else if (this.expanded) {
      const H = L.geometry.expandedHeight;
      const r = L.geometry.panelConcaveRadius;
      regions.push({ x: 0, y: 0, width: L.width, height: H });
      const drops = this.ordered();
      const final = computeFrame(L, 1, 1, this.bumps(drops, true));
      drops.forEach((d, i) => {
        if (!d.hasPanel || d.spring.target < 0.5) return;
        const bf = final.bumps[i];
        regions.push({ x: bf.left - r, y: H - 1, width: bf.right - bf.left + 2 * r, height: bf.depth - H + 1 });
      });
    }
    const key = JSON.stringify(regions);
    if (key === this.lastRegions) return;
    this.lastRegions = key;
    invoke("set_regions", { regions }).catch((e) => console.error("set_regions", e));
  }
}

// Surface frontend errors in the app log (the overlay has no visible devtools).
const report = (message: string) => invoke("fe_log", { message }).catch(() => {});

// Tooltips ([general] tooltips): widgets set `title`s; with tooltips off they're
// taken away as soon as they're set (native tooltips can't be styled away).
let tooltips = true;
const stripTitles = (root: Element) => root.querySelectorAll("[title]").forEach((e) => e.removeAttribute("title"));
new MutationObserver((records) => {
  if (tooltips) return;
  for (const r of records) {
    if (r.type === "attributes") (r.target as Element).removeAttribute("title");
    else r.addedNodes.forEach((n) => n instanceof Element && (n.removeAttribute("title"), stripTitles(n)));
  }
}).observe(document.documentElement, { subtree: true, childList: true, attributes: true, attributeFilter: ["title"] });

// Files dropped on a bar are for the tasks widget (if any): never open them as a page.
for (const type of ["dragover", "drop"]) document.addEventListener(type, (e) => e.preventDefault());
window.addEventListener("error", (e) => report(`${e.message} @ ${e.filename}:${e.lineno}`));
window.addEventListener("unhandledrejection", (e) => report(`unhandled: ${e.reason}`));

async function main(): Promise<void> {
  document.addEventListener("contextmenu", (e) => e.preventDefault());
  const overlay = new Overlay();
  const win = getCurrentWebviewWindow();
  await win.listen<Layout>("notch://layout", (e) => overlay.apply(e.payload));
  await win.listen<{ expanded: boolean }>("notch://state", (e) => overlay.setExpanded(e.payload.expanded));
  await win.listen<NotchLayout[]>("notch://positions", (e) => overlay.updatePositions(e.payload));
  await win.listen<{ open: boolean }>("notch://launcher", (e) => overlay.setLauncher(e.payload.open));
  await win.listen<{ code: string; name: string }>("notch://keyboard", (e) => overlay.showKeyboard(e.payload));
  await win.listen<{ kind: "volume" | "brightness"; level: number; muted: boolean }>("notch://osd", (e) =>
    overlay.showOsd(e.payload),
  );
  await initData().catch((e) => report(`data: ${e}`));
  overlay.apply(await invoke<Layout>("get_layout"));
}

main().catch((e) => report(`main: ${e}`));
