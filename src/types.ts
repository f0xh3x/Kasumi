// Mirrors engine::Layout in src-tauri/src/engine.rs. All values are logical px.

export interface Geometry {
  expandedHeight: number;
  cornerRadius: number;
  concaveRadius: number;
  notchHeight: number;
  notchCornerRadius: number;
  panelConcaveRadius: number;
  panelCornerRadius: number;
}

export interface Animation {
  expandStiffness: number;
  expandDamping: number;
  spreadStiffness: number;
  spreadDamping: number;
  panelStiffness: number;
  panelDamping: number;
  mass: number;
  style: "drop" | "spread";
  panelContent: "slide" | "reveal" | "fade";
  barContent: "slide" | "reveal" | "fade";
  openSpeed: number;
  closeSpeed: number;
  hoverNudge: boolean;
  cascade: boolean;
  rolling: boolean;
  magnify: boolean;
  markerGlide: boolean;
  notificationNudge: boolean;
  /** Springs may overshoot (as damped as configured); off: never past their target. */
  bounce: boolean;
}

export interface BarWidgets {
  left: string[];
  center: string[];
  right: string[];
  padding: number;
  gap: number;
}

export interface NotchLayout {
  id: string;
  x: number;
  width: number;
  height: number;
  triggerX: number;
  triggerWidth: number;
  widgets: string[];
  panel: string | null;
  panelWidth: number;
  panelHeight: number;
  /** Retracted: all its widgets hide under the window covering it. */
  hidden: boolean;
  /** Widgets hidden under a window while the rest of the notch stays. */
  hiddenWidgets: string[];
  /** Dock: no pinned widget in it, so it's only there while the dock is open. */
  closedHidden: boolean;
  /** Hugs this corner: content keeps the bar's distance from the edge. */
  edge: "left" | "right" | null;
  /** Dock: position and size while open. */
  openX: number;
  openWidth: number;
  openHeight: number;
  /** Dock: shrunk to the tiny handle (a window covers it). */
  shrunk: boolean;
}

export interface Osd {
  enabled: boolean;
  width: number;
  height: number;
  durationMs: number;
  keyboardWidth: number;
  /** New notifications pop out of the notch, in this style, for this long. */
  notifications: boolean;
  notificationStyle: "line" | "two" | "compact";
  notificationMs: number;
  notificationFit: boolean;
  notificationWidth: number;
}

export interface Theme {
  background: string;
  text: string;
  dimOpacity: number;
  accent: string;
  accentSync: boolean;
  font: string;
  fontSecondary: string;
  warn: string;
  accentDetails: boolean;
  fontSize: number;
  /** Opacity of the bars, notches and panels (percent). */
  opacity: number;
}

export type Edge = "top" | "bottom";
export type SurfaceMode = "bar" | "dock" | "always";
export type SizeName = "tiny" | "small" | "medium" | "large";

export interface Layout {
  /** Device name of the monitor (`\\.\DISPLAY4`). */
  device: string;
  /** The surface this overlay draws (`top`, `bottom`, ...). */
  surface: string;
  edge: Edge;
  mode: SurfaceMode;
  /** Dock sizes at rest and open (bar mode: both the notch height). */
  rest: SizeName;
  open: SizeName;
  restHeight: number;
  openHeight: number;
  label: string;
  monitor: string;
  width: number;
  height: number;
  scale: number;
  geometry: Geometry;
  animation: Animation;
  bar: BarWidgets;
  notches: NotchLayout[];
  widgets: Record<string, Record<string, unknown>>;
  osd: Osd;
  theme: Theme;
  /** Tooltips on widgets ([general] tooltips). */
  tooltips: boolean;
  /** The app launcher is switched on. */
  launcher: boolean;
  /** Rounded screen corners drawn with the shape (0: off). */
  cornerRadius: number;
  combineNotches: boolean;
  expanded: boolean;
}

export interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}
