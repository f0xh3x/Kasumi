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
  notch: string;
  width: number;
  height: number;
  durationMs: number;
  keyboardWidth: number;
}

export interface Theme {
  background: string;
  text: string;
  dimOpacity: number;
  accent: string;
  warn: string;
  fontSize: number;
}

export type Edge = "top" | "bottom";
export type SurfaceMode = "bar" | "dock";
export type SizeName = "tiny" | "small" | "medium";

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
  combineNotches: boolean;
  expanded: boolean;
}

export interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}
