import type { Layout } from "./types";

/**
 * The whole overlay is ONE closed path with a fixed node count:
 *
 *   hull   – the full-width bar. Collapsed it has zero depth and spans only the
 *            notches; opening, it drops to `expandedHeight` (driven by `t`) and
 *            spreads out to the screen edges (driven by `s`), with concave fillets
 *            at its ends that slide off-screen.
 *   bumps  – hanging from the hull's bottom edge: one per notch, plus one per bar
 *            widget that owns a dropdown. Collapsed, notch bumps are the notches;
 *            open, they flatten into the bar (depth == bar depth); a panel (`p`)
 *            makes a bump wider and deeper so it drops out of the bar.
 *
 * Every element is drawn as a "bump": concave fillet, wall, convex corner, floor,
 * convex corner, wall, concave fillet. Radii shrink to fit the available depth,
 * so a zero-depth bump degenerates into a straight line and the morph is seamless.
 *
 * Notch sides follow the reference profile (see `profileSide`): a gentle flare
 * from the edge, a slanted inflection, and a soft round-off into the tip. Popups
 * (panels, dropdowns) use circular fillets and corners (`roundedSide`).
 */

const OFF = 64; // how far outside the window the closing edges run

export interface Bump {
  /** Collapsed left edge and width (dropdowns of bar widgets: width 0 at the widget's center). */
  x: number;
  width: number;
  /** Collapsed depth (0 for bar dropdowns: they only exist while open). */
  height: number;
  panelWidth: number;
  panelHeight: number;
  /** Panel progress, 0 closed → 1 open. */
  p: number;
  /** Whether the collapsed hull should span this bump (notches yes, bar dropdowns no). */
  inHull: boolean;
  /** Style blend override: 0 = notch profile, 1 = rounded popup. Default: notches
   * morph by panel progress, everything else is rounded. */
  rounded?: number;
  /** Scale of the notch curves (docks: height / notch height), default 1. */
  scale?: number;
  /** Where an open panel starts (default: the bar's bottom edge). */
  panelTop?: number;
}

export interface BumpFrame {
  /** Center x of the bump / panel. */
  cx: number;
  /** Center x of the notch itself (where its own content stays while a panel is open). */
  anchorCx: number;
  left: number;
  right: number;
  /** Depth of the notch's content row (collapsed: notch height, open: bar height). */
  rowDepth: number;
  /** Total depth of the bump including any open panel. */
  depth: number;
}

export interface Frame {
  d: string;
  /** Current depth of the full-width bar. */
  base: number;
  bumps: BumpFrame[];
}

const f = (n: number) => (Math.round(n * 100) / 100).toString();
const lerp = (a: number, b: number, k: number) => a + (b - a) * k;
const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));

/**
 * The notch profile is taken from the reference notch (Asset 1.svg): a long gentle
 * concave flare, an inflection about halfway down with a steep slanted tangent,
 * then a soft convex round-off into the tip. Handle lengths are the reference's,
 * as ratios. Bumps deeper than the reference's aspect (panels) get a straight,
 * near-vertical wall between the two curves; shallower ones (mid-morph) are
 * squashed vertically. Both blends are continuous in depth.
 */
const REF_ASPECT = 40.17 / 78.33; // reference depth / (flare + round-off) width
const REF_SPLIT = 19.92 / 40.17; // inflection height / depth
const H_FLARE_START = 24.33 / 44.25; // flare handle at the edge, × flare width
const H_FLARE_END = 8.47 / 48.53; // flare handle at the inflection, × flare chord
const H_ROUND_START = 13.91 / 39.64; // round-off handle at the inflection, × its chord
const H_ROUND_END = 15.66 / 34.08; // round-off handle at the tip, × its width

const K = 0.5523; // cubic Bézier circle-arc constant (rounded style)

type Pt = [number, number];

/** One side of a bump, in traversal order from the edge above (p0) to the floor (p3). */
interface Side {
  p0: Pt; c1: Pt; c2: Pt; m1: Pt; // outer curve
  m2: Pt; c3: Pt; c4: Pt; p3: Pt; // wall to m2, then inner curve
}

/** Notch style: the reference profile. `x` is the inflection. */
function profileSide(x: number, top: number, bottom: number, r: number, c: number, dir: 1 | -1): Side {
  const D = Math.max(0, bottom - top);
  const natural = (r + c) * REF_ASPECT;
  const curveDepth = Math.min(D, natural);
  const gap = D - curveDepth; // straight wall, only when deeper than the reference shape
  const vr = curveDepth * REF_SPLIT;
  const vc = curveDepth - vr;
  // Inflection tangent ∝ (flare width, 2 × flare depth), tending to vertical as the wall grows.
  let tx = r;
  let ty = 2 * vr + gap * 20;
  const len = Math.hypot(tx, ty) || 1;
  tx = (dir * tx) / len;
  ty /= len;
  const hx = ty > 1e-6 ? (gap * Math.abs(tx)) / ty : 0;
  const h1 = H_FLARE_START * r;
  const h2 = H_FLARE_END * Math.hypot(r, vr);
  const h3 = H_ROUND_START * Math.hypot(c, vc);
  const h4 = H_ROUND_END * c;
  const x0 = x - dir * r;
  const x3 = x + dir * c;
  const m1: Pt = [x - (dir * hx) / 2, top + vr];
  const m2: Pt = [x + (dir * hx) / 2, bottom - vc];
  return {
    p0: [x0, top],
    c1: [x0 + dir * h1, top],
    c2: [m1[0] - tx * h2, m1[1] - ty * h2],
    m1,
    m2,
    c3: [m2[0] + tx * h3, m2[1] + ty * h3],
    c4: [x3 - dir * h4, bottom],
    p3: [x3, bottom],
  };
}

/** Popup style: circular concave fillet, straight wall at `x`, circular corner. */
function roundedSide(x: number, top: number, bottom: number, r: number, c: number, dir: 1 | -1): Side {
  return {
    p0: [x - dir * r, top],
    c1: [x - dir * r * (1 - K), top],
    c2: [x, top + r - r * K],
    m1: [x, top + r],
    m2: [x, bottom - c],
    c3: [x, bottom - c + c * K],
    c4: [x + dir * c * (1 - K), bottom],
    p3: [x + dir * c, bottom],
  };
}

/** Shrinks rounded-style radii so both fit a bump of the given depth and width. */
function fit(r: number, c: number, depth: number, width: number): [number, number] {
  const d = Math.max(0, depth);
  c = Math.min(c, Math.max(0, width) / 2);
  if (r + c > d) {
    const k = r + c > 0 ? d / (r + c) : 0;
    r *= k;
    c *= k;
  }
  return [r, c];
}

const mix = (a: Side, b: Side, w: number): Side => {
  if (w <= 0) return a;
  if (w >= 1) return b;
  const m = (p: Pt, q: Pt): Pt => [lerp(p[0], q[0], w), lerp(p[1], q[1], w)];
  return {
    p0: m(a.p0, b.p0), c1: m(a.c1, b.c1), c2: m(a.c2, b.c2), m1: m(a.m1, b.m1),
    m2: m(a.m2, b.m2), c3: m(a.c3, b.c3), c4: m(a.c4, b.c4), p3: m(a.p3, b.p3),
  };
};

const pt = (p: Pt) => `${f(p[0])} ${f(p[1])}`;

/** Left half of a bump: from the edge, down to the floor. */
function emitLeft(P: string[], s: Side) {
  P.push(`L${pt(s.p0)}`, `C${pt(s.c1)} ${pt(s.c2)} ${pt(s.m1)}`, `L${pt(s.m2)}`, `C${pt(s.c3)} ${pt(s.c4)} ${pt(s.p3)}`);
}

/** Right half of a bump: the same side traversed from the floor back up to the edge. */
function emitRight(P: string[], s: Side) {
  P.push(`L${pt(s.p3)}`, `C${pt(s.c4)} ${pt(s.c3)} ${pt(s.m2)}`, `L${pt(s.m1)}`, `C${pt(s.c2)} ${pt(s.c1)} ${pt(s.p0)}`);
}

/**
 * Both sides of a bump. `w` blends from the notch profile (0) to the rounded
 * popup style (1): a notch morphs into a rounded panel as it opens.
 */
function bump(P: string[], g: Layout["geometry"], left: number, right: number, top: number, bottom: number, w: number, k = 1) {
  const nc = Math.min(g.notchCornerRadius * k, Math.max(0, right - left) / 2);
  const cr = g.concaveRadius * k;
  const [pr, pc] = fit(g.panelConcaveRadius, g.panelCornerRadius, bottom - top, right - left);
  emitLeft(P, mix(profileSide(left, top, bottom, cr, nc, 1), roundedSide(left, top, bottom, pr, pc, 1), w));
  emitRight(P, mix(profileSide(right, top, bottom, cr, nc, -1), roundedSide(right, top, bottom, pr, pc, -1), w));
}

/**
 * @param t expand progress (0 collapsed → 1 open; may overshoot)
 * @param s spread progress (0 hull spans notches → 1 hull spans the screen)
 * @param bumps notches and dropdowns, sorted left to right
 */
export function computeFrame(L: Layout, t: number, s: number, bumps: Bump[]): Frame {
  const g = L.geometry;
  const W = L.width;
  const H = g.expandedHeight;
  const r = g.concaveRadius;
  const t0 = Math.max(0, t);
  const base = H * t0;

  const frames: BumpFrame[] = bumps.map((b) => {
    const p = Math.max(0, b.p);
    const rowDepth = lerp(b.height, H, t0);
    const depth = lerp(rowDepth, (b.panelTop ?? H) + b.panelHeight, p);
    // The panel is centered on its notch but kept on screen (flush with the edge
    // if need be). Each side of the bump morphs toward the panel's side, so a
    // notch in a corner grows its panel inward instead of sliding away.
    const anchorCx = b.x + b.width / 2;
    const panelLeft = clamp(anchorCx - b.panelWidth / 2, 0, Math.max(0, W - b.panelWidth));
    const left = lerp(anchorCx - b.width / 2, panelLeft, p);
    const right = lerp(anchorCx + b.width / 2, panelLeft + b.panelWidth, p);
    return { cx: (left + right) / 2, anchorCx, left, right, rowDepth, depth };
  });

  // Hull ends: collapsed they hug the outermost notches (leaving room for the fillets),
  // fully spread they sit on the screen edges.
  const hullBumps = bumps.filter((b) => b.inHull);
  const pad = r + g.cornerRadius + 1;
  const hl0 = hullBumps.length ? Math.min(...hullBumps.map((b) => b.x)) - pad : W / 2;
  const hr0 = hullBumps.length ? Math.max(...hullBumps.map((b) => b.x + b.width)) + pad : W / 2;
  const hl = lerp(hl0, Math.min(0, hl0), s);
  const hr = lerp(hr0, Math.max(W, hr0), s);
  // The hull's bottom corners are round while it spreads out of the notches and
  // square once it reaches the screen edges.
  const [hr_, hc] = fit(g.panelConcaveRadius, g.cornerRadius * clamp(1 - s, 0, 1), base, hr - hl);

  const P: string[] = [`M${-OFF} 0`];
  emitLeft(P, roundedSide(hl, 0, base, hr_, hc, 1));
  // Notches morph into rounded panels as they open; bar dropdowns are always rounded.
  const style = (i: number) => bumps[i].rounded ?? (bumps[i].inHull ? clamp(bumps[i].p, 0, 1) : 1);
  // Zero-size, flat bumps (closed dropdowns) draw nothing; skipping them lets
  // neighbouring notches see each other.
  const invisible = (i: number) => frames[i].right - frames[i].left < 1 && frames[i].depth - base < 0.5;
  const plainNotch = (i: number) => bumps[i].inHull && bumps[i].p <= 0.001;
  const MERGE_GAP = 6; // px between flares below which notches combine
  for (let i = 0; i < frames.length; ) {
    if (invisible(i)) {
      i++;
      continue;
    }
    let { left, right, depth } = frames[i];
    let k = bumps[i].scale ?? 1;
    let next = i + 1;
    if (L.combineNotches && plainNotch(i)) {
      // Notches that would touch or overlap become one shape; their widgets stay put.
      for (let j = i + 1; j < frames.length; j++) {
        if (invisible(j)) continue;
        if (!plainNotch(j) || frames[j].left - right - 2 * r * k > MERGE_GAP) break;
        right = Math.max(right, frames[j].right);
        k = Math.max(k, bumps[j].scale ?? 1);
        depth = Math.max(depth, frames[j].depth);
        next = j + 1;
      }
    }
    bump(P, g, left, right, base, depth, style(i), k);
    i = next;
  }
  emitRight(P, roundedSide(hr, 0, base, hr_, hc, -1));
  P.push(`L${f(W + OFF)} 0L${f(W + OFF)} ${-OFF}L${-OFF} ${-OFF}Z`);

  return { d: P.join(""), base, bumps: frames };
}
