import "@fontsource/inter/600.css";
import "./styles.css";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { initData } from "./data";
import { computeFrame, lerp, mirrorPath, type Bump } from "./geometry";
import { Spring } from "./spring";
import type { Layout, NotchLayout, Rect } from "./types";
import { appIcon, createPanel, createWidget, fontName, g, languageCode, panelFor, widgetType, type Widget } from "./widgets";
import { launcherHold, launcherSetOpen } from "./widgets/launcher";

const svg = document.getElementById("shape") as unknown as SVGSVGElement;
const fill = document.getElementById("fill") as unknown as SVGPathElement;
const cornersPath = document.getElementById("corners") as unknown as SVGPathElement;
const content = document.getElementById("content") as HTMLDivElement;

const clamp01 = (v: number) => Math.min(1, Math.max(0, v));

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
  root.setProperty("--font-size", `${t.fontSize}px`);
  // Installed fonts for main and secondary text ([theme] font / font_secondary); built-in when unset.
  for (const [prop, font] of [["--font", t.font], ["--font-secondary", t.fontSecondary]] as const) {
    if (font) root.setProperty(prop, fontName(font));
    else root.removeProperty(prop);
  }
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

/** [animation] keys behind each per-widget effect (`[widgets.<w>] anim_<key>`). */
const ANIM_GLOBAL = {
  cascade: "cascade",
  roll: "rolling",
  magnify: "magnify",
  glide: "markerGlide",
  nudge: "notificationNudge",
} as const;

/** The pieces a panel shows one after another (cascade), in page order. */
const PANEL_ROWS = [
  ".panel > .panel-calendar > :not(.cal-grid)",
  ".cal-grid > *",
  ".wx-head",
  ".wx-hours > *",
  ".menu-list > *",
  ".hw-cards > *",
  ".media-panel > *",
  ".claude-panel > *",
  ".note-list > *",
  ".note-actions",
  ".clip-list > *",
  ".privacy-panel > *",
  ".launcher-list > *",
  ".quick-grid > *",
  ".quick-tiles > *",
  ".quick-outputs > *",
  ".quick-apps > *",
  ".tray-grid > *",
  ".tasks-panel > *",
].join(", ");

/**
 * Cascade, by time (a spring is nearly open within ~100 ms, too fast to see one
 * piece follow another): each element fades and slides `shift` px into place a
 * little after the one before. Returns when the last one is in.
 */
function cascade(els: ArrayLike<HTMLElement>, start: number, shift: number): number {
  const n = els.length;
  const step = Math.min(25, 240 / Math.max(1, n));
  const dur = 160;
  const end = start + (n - 1) * step + dur;
  const t = performance.now() - start;
  for (let i = 0; i < n; i++) {
    const el = els[i];
    if (performance.now() >= end) {
      clearCascade(el);
      continue;
    }
    const k = clamp01((t - i * step) / dur);
    const e = 1 - (1 - k) ** 3;
    el.style.opacity = e.toFixed(3);
    el.style.translate = `0 ${Math.round((1 - e) * shift)}px`;
  }
  return end;
}

function clearCascade(el: HTMLElement): void {
  el.style.opacity = "";
  el.style.translate = "";
}

/** Space between a notch's content and the start of its curves, each side. */
const NOTCH_PAD = 10;
/**
 * Height of the drawing area (SVG and content). Oversized, so the window can grow
 * (panels) without resizing it; on a bottom surface it's anchored to the window's
 * bottom, so it doesn't move when the window grows upward either.
 */
const STAGE_HEIGHT = 2000;
/** Room a bottom bar's window keeps for its panels from the start (see measureBarDrops). */
const BOTTOM_PANEL_ROOM = 640;

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
  private osdTrack = div("osd-track");
  /** The notch the current pop-up grows out of (null: the center one). */
  private osdNotch: string | null = null;
  /** A notification's pill size while one is out (null: the level pill's, [osd] width / height). */
  private osdSize: { w: number; h: number } | null = null;
  /** Hover nudge: the notch the cursor comes near grows a little (Rust says which). */
  private nudge = new Spring(0, 0, 600, 30);
  private nudgeKey: string | null = null;
  /** When the panels' cascades began (by panel), and when time-driven effects end: keep drawing till then. */
  private cascadeStart = new Map<HTMLElement, number>();
  private holdUntil = 0;
  /** When a closed launcher started being held in the dock (0: it isn't). */
  private launcherHeld = 0;

  // Keyboard-layout peek: a small notch pops out at the language widget.
  private peek = new Spring(0, 0, 420, 32);
  private peekTimer = 0;
  private peekDrop: Drop | null = null;
  private peekEl = div("peek");
  private langEl: HTMLElement | null = null;

  // Notch widgets (one instance each: a notch's widgets aren't in the bar too).
  private notchEls: { slot: HTMLDivElement; name: string; el: HTMLElement }[] = [];

  private widgets: Widget[] = [];
  /** Opens the launcher's panel on this overlay (its widget's, or a fallback one). */
  private openLauncher: (() => void) | null = null;
  private bar = div("bar");
  private groups: Partial<Record<"left" | "center" | "right", HTMLDivElement>> = {};
  /** Bars: each pinned (notch) widget's place in its side's bar order, and its glide there. */
  private spacers = new Map<string, HTMLElement>();
  /** from: where it starts (its place in the closed notch); left: where it is in its slot now. */
  private glides: { el: HTMLElement; slot: HTMLDivElement; to: number; from: number; left: number }[] = [];
  /** The bar's own widgets that fade in behind pinned ones gliding past them. */
  private fadeIn: HTMLElement[] = [];
  /** Bar widgets' last x, so a move slides (flipBar). */
  private barX = new Map<HTMLElement, number>();
  private barObserver = new ResizeObserver(() => this.onBarResize());
  /** Center widgets around a middle notch: [before it, after it]. */
  private centerSplit: [HTMLDivElement, HTMLDivElement] | null = null;
  // Notch widths follow their content: measured here, positioned by Rust.
  private notchObserver = new ResizeObserver(() => {
    this.scheduleMeasure();
    this.onBarResize();
  });
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
    this.expand.speed(a.openSpeed, a.closeSpeed);
    this.spread.speed(a.openSpeed, a.closeSpeed);
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

    svg.setAttribute("width", String(layout.width));
    svg.setAttribute("height", String(STAGE_HEIGHT));
    svg.setAttribute("viewBox", `0 0 ${layout.width} ${STAGE_HEIGHT}`);
    // A bottom surface's content hangs from the window's bottom, as tall as the SVG.
    content.style.height = layout.edge === "bottom" ? `${STAGE_HEIGHT}px` : "";
    this.requestedHeight = 0;
    this.buildDom(layout, prevSprings, prevX);
    this.layoutBar();
    this.notchObserver.disconnect();
    for (const d of this.drops) for (const c of d.slot?.children ?? []) this.notchObserver.observe(c);
    this.barObserver.disconnect();
    for (const c of this.bar.querySelectorAll(".group > :not(.notch-spacer)")) this.barObserver.observe(c);
    this.scheduleMeasure();
    this.measureBarDrops();
    this.render();
    this.kick();
    this.sendRegions();
  }

  setExpanded(expanded: boolean): void {
    this.expanded = expanded;
    // Dock notches with no pinned widget come out as the dock opens and tuck away as it closes.
    for (const d of this.drops) if (d.shown && d.notch) d.shown.target = this.notchShown(d.notch) ? 1 : 0;
    if (expanded) this.nudge.target = 0;
    // Open-only parts come at once; they go once the bar has closed (render), the
    // shrinking shape covering them meanwhile, so nothing pops as it closes.
    if (expanded) document.documentElement.classList.add("bar-open");
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
      if (d?.shown) d.shown.target = this.notchShown(n) ? 1 : 0;
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
    const sideOf = (n: NotchLayout) => (["left", "center", "right"] as const).find((s) => n.widgets.some((w) => L.bar[s].includes(w)));
    let middle: NotchLayout | null = null;
    for (const n of L.notches) if (!n.hidden && n.widgets.length && sideOf(n) === "center") middle = n;
    if (this.centerSplit) {
      const [before, after] = this.centerSplit;
      const x0 = middle ? middle.x + inset : L.width / 2;
      const x1 = middle ? middle.x + middle.width - inset : L.width / 2;
      before.style.right = `${L.width - x0 + gap}px`;
      after.style.left = `${x1 + gap}px`;
    }
    // Left / right: a pinned widget keeps its place in the bar's order (its spacer, as
    // wide as it is); render() glides it there from its notch as the bar opens.
    for (const [name, sp] of this.spacers) {
      const e = this.notchEls.find((x) => x.name === name);
      const n = e && L.notches.find((x) => x.id === e.slot.dataset.notch);
      const shown = !!e && !!n && !n.hidden && !n.hiddenWidgets.includes(name) && e.el.offsetWidth > 0;
      sp.style.display = shown ? "" : "none";
      if (e && shown) sp.style.width = `${e.el.offsetWidth}px`;
    }
    const barLeft = this.bar.getBoundingClientRect().left;
    // Open-only parts (the media keys) widen a notch's content past its closed width,
    // which re-centers everything in it. The glides start where each widget was in the
    // closed notch (read with the bar closed; nothing is painted in between). A widget
    // that grew is placed by a part of it seen both ways (the media title, not its keys).
    const html = document.documentElement;
    type Seen = { left: number; width: number; kids: [number, number][] };
    const look = (el: HTMLElement): Seen => {
      const r = el.getBoundingClientRect();
      const kids = Array.from(el.children, (c): [number, number] => {
        const k = c.getBoundingClientRect();
        return [k.left - r.left, k.width];
      });
      return { left: el.offsetLeft, width: el.offsetWidth, kids };
    };
    const closed = new Map<HTMLElement, Seen>();
    if (html.classList.contains("bar-open")) {
      html.classList.remove("bar-open");
      for (const e of this.notchEls) closed.set(e.el, look(e.el));
      html.classList.add("bar-open");
    }
    const start = (el: HTMLElement): number => {
      const c = closed.get(el);
      if (!c) return el.offsetLeft;
      const now = look(el);
      if (Math.abs(c.width - now.width) <= 1) return c.left;
      const i = c.kids.findIndex(([, w], j) => w > 0 && now.kids[j] && Math.abs(now.kids[j][1] - w) <= 1);
      return i < 0 ? c.left : c.left + c.kids[i][0] - now.kids[i][0];
    };
    this.glides = [];
    for (const [name, sp] of this.spacers) {
      const e = this.notchEls.find((x) => x.name === name);
      if (!e || sp.style.display === "none") continue;
      this.glides.push({ el: e.el, slot: e.slot, to: sp.getBoundingClientRect().left - barLeft, from: start(e.el), left: e.el.offsetLeft });
    }
    // The bar's own widgets on a side where pinned ones glide in fade in behind them.
    this.fadeIn = (["left", "right"] as const)
      .map((side) => this.groups[side])
      .filter((g): g is HTMLDivElement => !!g && this.glides.some((x) => this.spacers.get(this.notchEls.find((e) => e.el === x.el)?.name ?? "")?.parentElement === g))
      .flatMap((g) => Array.from(g.children).filter((c): c is HTMLElement => !c.classList.contains("notch-spacer")));
    this.kick();
  }

  /** Bar widgets resized: re-place the pinned ones, and slide whatever moved. */
  private onBarResize(): void {
    if (!this.layout || this.layout.mode === "dock") return;
    this.layoutBar();
    this.flipBar();
  }

  /**
   * Widgets in the open bar slide to a new place instead of jumping (a title got
   * longer, a widget appeared): from where they were (layout x, transforms aside).
   */
  private flipBar(): void {
    const visible = this.expanded && this.expand.value > 0.5;
    for (const el of this.bar.querySelectorAll<HTMLElement>(".group > :not(.notch-spacer)")) {
      const x = el.offsetLeft;
      const was = this.barX.get(el);
      this.barX.set(el, x);
      if (visible && was !== undefined && Math.abs(was - x) > 1) {
        el.animate([{ translate: `${was - x}px 0` }, { translate: "0 0" }], { duration: 220, easing: "cubic-bezier(0.2, 0.8, 0.2, 1)" });
      }
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
    // A dock keeps showing the search while it closes, then its items come back (releaseLauncher).
    if (!open && this.layout?.mode === "dock" && document.querySelector(".launcher-w.searching")) {
      launcherHold(true);
      this.launcherHeld = performance.now();
      this.holdUntil = Math.max(this.holdUntil, this.launcherHeld + 1500);
      this.kick();
    } else if (open) {
      this.releaseLauncher(true);
    }
    launcherSetOpen(open);
  }

  /** Lets a held launcher go: once the dock has (nearly) closed, or it stays open after all. */
  private releaseLauncher(now = false): void {
    if (!this.launcherHeld) return;
    const since = performance.now() - this.launcherHeld;
    if (now || this.expand.value < 0.05 || (since > 150 && this.expand.target > 0.5) || since > 1500) {
      this.launcherHeld = 0;
      launcherHold(false);
    }
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
    // Redraw the shape now, not next frame: content animating inside a panel
    // (a list sliding open) would otherwise poke out of it for a frame each step.
    this.render();
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
    this.notchEls = [];
    this.spacers.clear();
    this.glides = [];
    this.fadeIn = [];
    this.barX.clear();
    this.openLauncher = null;
    this.lastWidths = "";
    content.replaceChildren();

    const a = L.animation;
    const spring = (key: string) => {
      const sp = prevSprings.get(key) ?? new Spring(0, 0, a.panelStiffness, a.panelDamping, a.mass);
      sp.configure(a.panelStiffness, a.panelDamping, a.mass);
      sp.speed(a.openSpeed, a.closeSpeed);
      return sp;
    };
    const ctx = { close: this.closeAll, resize: () => this.resizeDrops() };
    const opts = (name: string): Record<string, unknown> => ({
      ...(L.widgets[name] ?? {}),
      _device: L.device,
      _roll: this.animFor(name, "roll"),
      _magnify: this.animFor(name, "magnify"),
      _glide: this.animFor(name, "glide"),
      _accent: this.accentFor(name),
      _font: L.widgets[name.replace(/^calendar/, "clock")]?.font ?? "",
      _font2: L.widgets[name.replace(/^calendar/, "clock")]?.font_secondary ?? "",
      _nudge: this.animFor(name, "nudge"),
    });
    const mountPanel = (name: string, width?: number, height?: number, extra: Record<string, unknown> = {}) => {
      const panel = div(`panel panel-${widgetType(name)}`);
      // Its open effect (render() reads it): cascade.
      if (this.animFor(name, "cascade")) panel.dataset.cascade = "1";
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
        if (notched.has(name)) {
          // A pinned widget lives in its notch; this keeps its place in the open bar's order.
          const sp = div("notch-spacer");
          group.append(sp);
          this.spacers.set(name, sp);
          continue;
        }
        const w = createWidget(name, opts(name));
        this.widgets.push(w);
        group.append(w.el);
        if (name === "language") this.langEl = w.el;
        dropdown(name, w);
      }
      return group;
    };
    this.centerSplit = null;
    for (const side of ["left", "center", "right"] as const) {
      // Left / right: pinned ones too, as spacers keeping their place in the order.
      const names = side === "center" ? L.bar[side].filter((n) => !notched.has(n)) : L.bar[side];
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
        // The widget itself: a notch widget isn't in the bar too (one instance).
        const w = createWidget(name, opts(name));
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
      if (n.widgets.some((w) => widgetType(w) === "workspaces")) slot.classList.add("clickable");
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
        shown: new Spring(this.notchShown(n) ? 1 : 0, this.notchShown(n) ? 1 : 0, a.spreadStiffness, a.spreadDamping, a.mass),
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
          // The search field takes the dock's place while the launcher is open: it
          // shows only then, and its notch's other widgets step aside (styles.css).
          const inline = createWidget("launcher", opts("launcher"));
          inline.el.classList.add("launcher-inline");
          this.widgets.push(inline);
          middle.slot?.append(inline.el);
          const panel = mountPanel("launcher");
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
    this.osdTrack = div("osd-track");
    this.osdTrack.append(this.osdFill);
    this.osdSize = null;
    this.osdEl = div("osd");
    this.osdEl.append(this.osdIcon, this.osdTrack, this.osdValue);
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
    // An open bar shows open-only parts (media keys): its notches are measured closed.
    if (L.mode !== "dock" && document.documentElement.classList.contains("bar-open")) return;
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
    // Bars: a notch's width is its closed one, measured as if the bar were closed (an
    // open bar shows parts that are only there while it's open, like the media keys);
    // else every open and close would resize it, mid-animation. Not painted.
    const html = document.documentElement;
    const reopen = !dock && html.classList.contains("bar-open");
    if (reopen) html.classList.remove("bar-open");
    for (const n of L.notches) {
      const slot = this.drops.find((d) => d.key === `notch:${n.id}`)?.slot;
      if (!slot) continue;
      const measure = (h: number, t: number) => at(h, t, () => this.slotWidths(n, slot, base(h)));
      const [full, compact] = measure(L.restHeight, 0);
      const [open] = dock ? measure(L.openHeight, 1) : [full];
      if (full || open) widths[n.id] = { full, compact, open };
    }
    if (reopen) html.classList.add("bar-open");
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
        // Fractional, like the content: whole-pixel heights made the shape and the
        // content round apart by up to a pixel each frame (a tremble while resizing).
        d.panelEl.style.height = "auto";
        d.panelHeight = d.panelEl.getBoundingClientRect().height;
        d.panelEl.style.height = `${d.panelHeight}px`;
        tallest = Math.max(tallest, d.panelHeight);
        continue;
      }
      const size = d.panelEl.getBoundingClientRect();
      d.panelWidth = size.width;
      d.panelHeight = size.height;
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
    // A dock's notch panels that aren't open count too: the window must already be
    // tall enough when one opens (growing it then moves a bottom bar: a flicker).
    for (const d of this.drops) {
      for (const p of d.panels?.values() ?? []) {
        if (p !== d.panelEl) tallest = Math.max(tallest, p.offsetHeight);
      }
    }
    // A bottom bar's window grows upward: Windows moves its top edge and the page is
    // drawn at the old spot for a frame or two (the whole bar jumps). So it starts
    // with generous room (panels measured before their content arrives come out short).
    if (L.edge === "bottom" && this.drops.some((d) => d.panelEl || d.panels?.size)) tallest = Math.max(tallest + 80, BOTTOM_PANEL_ROOM);
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
  /**
   * Whether an animation effect is on for a widget (or its panel: "calendar" is
   * the clock's): its own `anim_<key>` in [widgets.<name>], else [animation]'s.
   */
  private animFor(name: string, key: keyof typeof ANIM_GLOBAL): boolean {
    const L = this.layout;
    if (!L) return false;
    const owner = name.replace(/^calendar/, "clock");
    const own = L.widgets[owner]?.[`anim_${key}`];
    return typeof own === "boolean" ? own : L.animation[ANIM_GLOBAL[key]] === true;
  }

  /** Accent on a widget's details: its own `accent_details`, else [theme]'s. */
  private accentFor(name: string): boolean {
    const L = this.layout;
    if (!L) return false;
    const own = L.widgets[name.replace(/^calendar/, "clock")]?.accent_details;
    return typeof own === "boolean" ? own : L.theme.accentDetails === true;
  }

  /** Hover nudge: Rust says which notch the cursor is coming near (null: none). */
  setNudge(notch: string | null): void {
    if (notch) {
      if (this.nudgeKey !== `notch:${notch}`) this.nudge.value = 0;
      this.nudgeKey = `notch:${notch}`;
    }
    this.nudge.target = notch ? 1 : 0;
    this.kick();
  }

  /** A panel's cascade: starts as it begins to open, reset once it's shut. */
  private cascadePanel(panel: HTMLElement, p: number, target: number, away: number): void {
    const rows = panel.querySelectorAll<HTMLElement>(PANEL_ROWS);
    let start = this.cascadeStart.get(panel);
    if (p <= 0.01 && target < 0.5) {
      if (start !== undefined) rows.forEach(clearCascade);
      this.cascadeStart.delete(panel);
      return;
    }
    if (start === undefined) {
      start = performance.now();
      this.cascadeStart.set(panel, start);
    }
    this.holdUntil = Math.max(this.holdUntil, cascade(rows, start, -8 * away));
  }

  /** Whether a notch shows now: not retracted under a window, and (dock) pinned or the dock open. */
  private notchShown(n: NotchLayout): boolean {
    return !n.hidden && !(n.closedHidden && !this.expanded);
  }

  /** The corner a notch drop hugs ("left" / "right"), null for the others. */
  private notchSide(d: Drop): "left" | "right" | null {
    return this.layout?.notches.find((n) => d.key === `notch:${n.id}`)?.edge ?? null;
  }

  private osdDrop(): Drop | undefined {
    const L = this.layout;
    if (!L) return undefined;
    // The notch the pop-up came for (Rust picks it: [osd] spot), else the center one.
    return (
      (this.osdNotch !== null ? this.drops.find((d) => d.key === `notch:${this.osdNotch}`) : undefined) ??
      this.drops.find((d) => d.key === "notch:center") ??
      this.drops.find((d) => d.inHull)
    );
  }

  /** Shows a level on the OSD notch, then shrinks back after the configured duration. */
  showOsd(e: { kind: "volume" | "brightness" | "keyboard" | "caps" | "update"; level: number; muted: boolean; text?: string; ms?: number; notch?: string | null }): void {
    const L = this.layout;
    this.osdNotch = e.notch ?? null;
    if (!L || !this.osdDrop()) return;
    // After a notification: the level pill's own content and size again.
    if (this.osdSize) {
      this.osdSize = null;
      this.osdEl.className = "osd";
      this.osdEl.replaceChildren(this.osdIcon, this.osdTrack, this.osdValue);
    }
    const pct = Math.round(e.level * 100);
    let glyph = 0xe706; // Brightness
    if (e.kind === "keyboard" || e.kind === "caps") glyph = 0xe765;
    if (e.kind === "update") glyph = 0xe896; // Download
    if (e.kind === "volume") {
      glyph = e.muted ? 0xe74f : pct === 0 ? 0xe992 : pct < 34 ? 0xe993 : pct < 67 ? 0xe994 : 0xe995;
    }
    this.osdIcon.textContent = g(glyph);
    this.osdFill.style.setProperty("--v", String(pct / 100));
    this.osdEl.classList.toggle("muted", e.muted);
    // "Update is available" (updater.rs): in the accent colour.
    this.osdEl.classList.toggle("update", e.kind === "update");
    this.osdValue.textContent = e.text ?? String(pct);
    this.osdEl.classList.toggle("text", e.text !== undefined);
    if (e.text !== undefined) {
      // Text (keyboard layout, Caps Lock): a pill around the text, not the level pill's width.
      const was = this.osdEl.style.width;
      this.osdEl.style.width = "max-content";
      const text = Math.ceil(this.osdEl.getBoundingClientRect().width);
      this.osdEl.style.width = was;
      this.osdSize = { w: Math.round(Math.max(100, text + 2 * L.geometry.notchCornerRadius + 8)), h: L.osd.height };
    }
    this.osd.target = 1;
    clearTimeout(this.osdTimer);
    this.osdTimer = window.setTimeout(() => {
      this.osd.target = 0;
      this.kick();
    }, e.ms ?? L.osd.durationMs);
    this.kick();
  }

  /**
   * A new notification out of the OSD notch ([osd] notifications): its app's icon
   * and name with the title and text on one line ("line"), the title over the
   * text ("two"), or just icon and app ("compact"), for notification_ms.
   */
  showNotification(n: { app: string; aumid: string; title: string; body: string; notch?: string | null }): void {
    const L = this.layout;
    this.osdNotch = n.notch ?? null;
    if (!L || !this.osdDrop()) return;
    const style = L.osd.notificationStyle === "two" || L.osd.notificationStyle === "compact" ? L.osd.notificationStyle : "line";
    const pic = div("osd-note-icon");
    const bell = document.createElement("span");
    bell.className = "icon";
    bell.textContent = g(0xea8f);
    pic.append(bell);
    appIcon(n.aumid).then((url) => {
      if (!url) return;
      const img = document.createElement("img");
      img.src = url;
      img.draggable = false;
      pic.replaceChildren(img);
    });
    const text = (cls: string, t: string) => {
      const d = div(cls);
      d.textContent = t;
      return d;
    };
    let parts: HTMLElement[];
    if (style === "compact") {
      parts = [pic, text("osd-note-app", n.app)];
    } else if (style === "two") {
      const lines = div("osd-note-lines");
      lines.append(text("osd-note-head", [n.app, n.title].filter(Boolean).join(" · ")), text("osd-note-text", n.body || n.title));
      parts = [pic, lines];
    } else {
      parts = [pic, text("osd-note-app", n.app), text("osd-note-text", [n.title, n.body].filter(Boolean).join(" — "))];
    }
    if (this.animFor("notifications", "nudge")) pic.classList.add("bounce");
    this.osdEl.className = `osd note note-${style}`;
    this.osdEl.replaceChildren(...parts);
    // The pill around the text (osd inner width = w - 2 * corner radius - 8), or always the widest.
    const widest = Math.max(L.osd.notificationWidth || 420, 160);
    let w = widest;
    if (L.osd.notificationFit !== false) {
      const was = this.osdEl.style.width;
      this.osdEl.style.width = "max-content";
      const text = Math.ceil(this.osdEl.getBoundingClientRect().width);
      this.osdEl.style.width = was;
      w = Math.round(Math.min(widest, Math.max(160, text + 2 * L.geometry.notchCornerRadius + 8)));
    }
    this.osdSize = { w, h: style === "two" ? L.osd.height + 20 : L.osd.height };
    this.osd.target = 1;
    clearTimeout(this.osdTimer);
    this.osdTimer = window.setTimeout(() => {
      this.osd.target = 0;
      this.kick();
    }, L.osd.notificationMs || 5000);
    this.kick();
  }

  /** Keyboard layout changed: pop a small notch at the language widget (or use the OSD). */
  showKeyboard(e: { code: string; code2?: string; name: string; notch?: string | null }): void {
    const L = this.layout;
    if (!L) return;
    // Out of a notch like the other pop-ups ([osd] keyboard_spot), not the peek.
    if (e.notch) {
      this.showOsd({ kind: "keyboard", level: 0, muted: false, text: languageCode(e, L.widgets.language), notch: e.notch });
      return;
    }
    // The language widget shows the new layout itself whenever it's visible.
    if (this.languageVisible()) return;
    const anchor = this.langEl?.getBoundingClientRect();
    // Docks (tiny handle / covered): grow the OSD pill rather than a second bump.
    if (!anchor || !this.peekDrop || this.expanded || L.mode === "dock") {
      this.showOsd({ kind: "keyboard", level: 0, muted: false, text: languageCode(e, L.widgets.language) });
      return;
    }
    this.peekDrop.x = anchor.left + anchor.width / 2;
    const glyph = document.createElement("span");
    glyph.className = "icon";
    glyph.textContent = g(0xe765);
    const code = document.createElement("span");
    code.textContent = languageCode(e, L.widgets.language);
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

  /** How far a notch's widgets move from their notch to their places in the open bar (on average). */
  private glideShift(d: Drop, x: number): number {
    const mine = d.slot ? this.glides.filter((g) => g.slot === d.slot) : [];
    return mine.length ? mine.reduce((sum, g) => sum + g.to - (x + g.from), 0) / mine.length : 0;
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
      // Bars: a notch whose widgets glide to their place in the open bar goes along
      // (its panel then drops below them).
      if (L.mode !== "dock") x += this.glideShift(d, x) * clamp01(useTarget ? this.expand.target : this.expand.value);
      let width = d.width;
      let height = d.height;
      // Hover nudge: the notch the cursor comes near grows a little (a dock opens a little).
      // Not cut off when the bar opens: it fades on its own spring as the opening takes over.
      const nudged = d.key === this.nudgeKey ? Math.max(0, useTarget ? this.nudge.target : this.nudge.value) : 0;
      let scale: number | undefined;
      let panelTop: number | undefined;
      if (L.mode === "dock" && d.notch) {
        // Grow from the rest size to the open size in place.
        const e = Math.min(1, Math.max(0, useTarget ? this.expand.target : this.expand.value, 0.15 * nudged));
        const n = d.notch;
        x = lerp(x, n.openX, e);
        width = lerp(n.width, n.openWidth, e);
        height = lerp(n.height, n.openHeight, e);
        scale = height / L.geometry.notchHeight;
        panelTop = height;
      } else if (nudged > 0) {
        x -= 5 * nudged;
        width += 10 * nudged;
        height += 4 * nudged;
      }
      if (d.shown) {
        // Retracting (hidden under a window): shrink into the top edge around the center.
        const s = clamp01(useTarget ? d.shown.target : d.shown.value);
        x += (width * (1 - s)) / 2;
        width *= s;
        height *= s;
      }
      if (d === od && o > 0) {
        // Grow from the notch's side: a corner notch keeps its outer edge (so the
        // pill stays on screen), the others their center.
        const side = this.notchSide(d);
        const from = x;
        width = d.width + ((this.osdSize?.w ?? L.osd.width) - d.width) * o;
        height = d.height + ((this.osdSize?.h ?? L.osd.height) - d.height) * o;
        // (A corner notch reaches past the screen edge, its round-off hidden there: keep that.)
        x = side === "left" ? from : side === "right" ? from + d.width - width : Math.max(0, Math.min(from + d.width / 2 - width / 2, L.width - width));
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

    // Positions below are distances from the surface's edge. A top surface lays
    // things out from the top; a bottom one from the bottom (styles.css anchors
    // them there), so moving away from the edge is up: y goes negative. Content is
    // never mirrored (a mirrored box redraws its text at sub-pixel offsets whenever
    // its height changes); only the SVG shape is, being pure vector.
    const bottom = L.edge === "bottom";
    const away = bottom ? -1 : 1;
    fill.setAttribute("d", frame.d);
    cornersPath.setAttribute("d", frame.corners);
    content.style.clipPath = `path("${bottom ? mirrorPath(frame.d, STAGE_HEIGHT) : frame.d}")`;

    // slide: the row rides the bar's far edge. reveal / fade: it waits in its open
    // spot and the growing shape (content's clip) uncovers it; reveal shows it at
    // once (just enough fade that nothing flashes inside the notches' flares).
    const barHow = L.animation.barContent;
    const ex = this.expand.value;
    if (!this.expanded && ex < 0.01 && document.documentElement.classList.contains("bar-open")) {
      document.documentElement.classList.remove("bar-open");
      this.layoutBar();
    }
    this.bar.style.transform = barHow === "slide" ? `translateY(${(frame.base - H) * away}px)` : "none";
    this.bar.style.opacity = String(barHow === "reveal" ? clamp01(ex * 5) : barHow === "fade" ? clamp01((ex - 0.35) / 0.55) : clamp01((ex - 0.25) / 0.6));

    if (!dock) {
      const fade = ex >= 0.999 ? "" : clamp01((ex - 0.5) / 0.5).toFixed(3);
      for (const el of this.fadeIn) el.style.opacity = fade;
    }

    const od = this.osdDrop();
    const o = clamp01(this.osd.value);
    drops.forEach((d, i) => {
      const bf = frame.bumps[i];
      // The notch row's depth in whole pixels: content is centered in it, and the
      // spring's last tiny wobble around the final depth would otherwise move an
      // item whose centered position sits on a half pixel back and forth by 1 px.
      const row = Math.round(bf.rowDepth);
      if (d.slot) {
        // Docks: the slot follows the notch's current (morphing) width.
        const w = dock ? bumps[i].width : d.width;
        d.slot.style.height = `${row}px`;
        d.slot.style.transform = `translateX(${bf.anchorCx - w / 2}px)`;
        if (!dock) {
          // Pinned widgets glide from the notch to their place in the bar's order
          // (the notch moves along by their average; each makes up the rest).
          const slotX = bf.anchorCx - w / 2;
          const rest = d.xs ? d.xs.value : d.x;
          const e = clamp01(this.expand.value);
          for (const g of this.glides) {
            if (g.slot !== d.slot) continue;
            const at = lerp(rest + g.from, g.to, e);
            // Also at the very start: open-only parts have already re-centered the slot.
            const dx = Math.round(at - (slotX + g.left));
            g.el.style.translate = dx ? `${dx}px 0` : "";
          }
        }
        if (dock) d.slot.style.width = `${w}px`;
        // Cross-fade the notch's own content with the OSD; a tiny dock handle hides it.
        const shown = d.shown ? clamp01(d.shown.value * 1.5 - 0.5) : 1;
        const fits = dock ? clamp01((bf.rowDepth - 16) / 10) : 1;
        d.slot.style.opacity = String(shown * fits * (d === od ? 1 - clamp01(o * 1.8) : 1));
      }
      if (d === this.peekDrop) {
        const k = clamp01(this.peek.value);
        this.peekEl.style.transform = `translate(${bf.cx}px, ${(away * row) / 2}px) translate(-50%, ${-away * 50}%)`;
        this.peekEl.style.opacity = String(clamp01(k * 1.8 - 0.5) * (1 - clamp01(this.expand.value)));
        this.peekEl.style.visibility = k > 0.01 ? "visible" : "hidden";
      }
      if (d === od) {
        // Content lines up with the side the pill grows from.
        const side = this.notchSide(d);
        this.osdEl.classList.toggle("from-left", side === "left");
        this.osdEl.classList.toggle("from-right", side === "right");
        const inner = (this.osdSize?.w ?? L.osd.width) - 2 * L.geometry.notchCornerRadius - 8;
        this.osdEl.style.width = `${inner}px`;
        this.osdEl.style.transform = `translate(${bf.cx - inner / 2}px, ${(away * row) / 2}px) translateY(${-away * 50}%)`;
        this.osdEl.style.opacity = String(clamp01(o * 1.8 - 0.5));
        this.osdEl.style.visibility = o > 0.01 ? "visible" : "hidden";
      }
      if (d.panelEl) {
        const p = d.spring.value;
        const shown = p > 0.01;
        // Panels hear when they're shut (notch:panel-closed, detail: the panel's box).
        if (!shown && d.panelEl.style.visibility === "visible") window.dispatchEvent(new CustomEvent("notch:panel-closed", { detail: d.panelEl }));
        d.panelEl.style.visibility = shown ? "visible" : "hidden";
        // slide: the content rides the panel's far edge (`depth`). reveal / fade: it waits
        // in its open spot and the growing shape (content's clip) uncovers it.
        const how = L.animation.panelContent;
        const near = how === "reveal" || how === "fade" ? (bumps[i].panelTop ?? H) : bf.depth - d.panelHeight;
        d.panelEl.style.opacity = String(how === "reveal" ? 1 : how === "fade" ? clamp01(p * 1.6 - 0.5) : clamp01(p * 1.4 - 0.2));
        d.panelEl.style.transform = `translate(${bf.cx - d.panelWidth / 2}px, ${(how === "slide" ? near : Math.round(near)) * away}px)`;
        // Cascade ([animation] cascade, or the widget's own).
        if (d.panelEl.dataset.cascade === "1") this.cascadePanel(d.panelEl, p, d.spring.target, away);
      }
    });
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
      this.nudge,
      ...this.drops.flatMap((d) => [d.spring, ...(d.xs ? [d.xs] : []), ...(d.shown ? [d.shown] : [])]),
    ];
    for (const s of springs) s.step(dt);
    this.releaseLauncher();
    // Time-driven effects (cascades) keep the loop going until they're done.
    if (springs.every((s) => s.settled) && now >= this.holdUntil) {
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

async function main(): Promise<void> {
  document.addEventListener("contextmenu", (e) => e.preventDefault());
  const overlay = new Overlay();
  const win = getCurrentWebviewWindow();
  await win.listen<Layout>("notch://layout", (e) => overlay.apply(e.payload));
  await win.listen<{ expanded: boolean }>("notch://state", (e) => overlay.setExpanded(e.payload.expanded));
  await win.listen<NotchLayout[]>("notch://positions", (e) => overlay.updatePositions(e.payload));
  await win.listen<{ open: boolean }>("notch://launcher", (e) => overlay.setLauncher(e.payload.open));
  await win.listen<{ code: string; code2?: string; name: string; notch?: string | null }>("notch://keyboard", (e) => overlay.showKeyboard(e.payload));
  await win.listen<{ kind: "volume" | "brightness" | "caps" | "update"; level: number; muted: boolean; text?: string; ms?: number; notch?: string | null }>("notch://osd", (e) =>
    overlay.showOsd(e.payload),
  );
  await win.listen<{ notch: string | null }>("notch://nudge", (e) => overlay.setNudge(e.payload.notch));
  await win.listen<{ app: string; aumid: string; title: string; body: string; notch?: string | null }>("notch://notification", (e) => overlay.showNotification(e.payload));
  await initData().catch((e) => report(`data: ${e}`));
  overlay.apply(await invoke<Layout>("get_layout"));
}

main().catch((e) => report(`main: ${e}`));
