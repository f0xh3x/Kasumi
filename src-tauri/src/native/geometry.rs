//! The one shape, ported line by line from src/geometry.ts (`computeFrame`): the
//! hull (full-width bar), notches and dropdowns hanging from it, notches morphing
//! into rounded panels, merging, stepped dock-corner panels, rounded screen
//! corners. Logical px, measured from the surface's own edge. Change both together.

use crate::config::Geometry;

pub const OFF: f64 = 64.0; // how far outside the window the closing edges run

// The reference notch's profile (Asset 1.svg).
const REF_ASPECT: f64 = 40.17 / 78.33;
const REF_SPLIT: f64 = 19.92 / 40.17;
const H_FLARE_START: f64 = 24.33 / 44.25;
const H_FLARE_END: f64 = 8.47 / 48.53;
const H_ROUND_START: f64 = 13.91 / 39.64;
const H_ROUND_END: f64 = 15.66 / 34.08;
const K: f64 = 0.5523; // cubic Bézier circle-arc constant

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pt(pub f64, pub f64);

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Cmd {
    M(Pt),
    L(Pt),
    C(Pt, Pt, Pt),
    Z,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Left,
    Right,
}

/// A notch (in the hull) or a bar widget's dropdown (width 0 at its widget's centre).
#[derive(Clone, Debug)]
pub struct Bump {
    pub x: f64,
    pub width: f64,
    pub height: f64,
    pub panel_width: f64,
    pub panel_height: f64,
    /// Panel progress, 0 closed to 1 open.
    pub p: f64,
    pub in_hull: bool,
    /// Style blend override: 0 notch profile, 1 rounded popup (default: notches
    /// morph by panel progress, everything else is rounded).
    pub rounded: Option<f64>,
    /// Scale of the notch curves (docks: height / notch height).
    pub scale: f64,
    /// Where an open panel starts (default: the bar's bottom edge).
    pub panel_top: Option<f64>,
    /// Docks: a corner notch's narrower panel rising from its corner end.
    pub step: Option<Step>,
}

impl Default for Bump {
    fn default() -> Self {
        Self { x: 0.0, width: 0.0, height: 0.0, panel_width: 0.0, panel_height: 0.0, p: 0.0, in_hull: false, rounded: None, scale: 1.0, panel_top: None, step: None }
    }
}

#[derive(Clone, Debug, Default)]
pub struct BumpFrame {
    /// Center x of the bump / panel.
    pub cx: f64,
    /// Center x of the notch itself (where its content stays while a panel is open).
    pub anchor_cx: f64,
    pub left: f64,
    pub right: f64,
    /// Depth of the notch's content row (collapsed: notch height, open: bar height).
    pub row_depth: f64,
    /// Total depth including any open panel.
    pub depth: f64,
    /// A stepped panel's sides (`left` / `right` are then the notch's).
    pub panel: Option<(f64, f64)>,
    /// Where the open panel's left edge ends up (its content waits there).
    pub panel_x: f64,
}

#[derive(Clone, Debug, Default)]
pub struct Frame {
    pub path: Vec<Cmd>,
    /// The rounded screen corners (drawn solid).
    pub corners: Vec<Cmd>,
    /// Current depth of the full-width bar.
    pub base: f64,
    pub bumps: Vec<BumpFrame>,
}

pub fn lerp(a: f64, b: f64, k: f64) -> f64 {
    a + (b - a) * k
}

/// One side of a bump, from the edge above (p0) to the floor (p3).
#[derive(Clone, Copy)]
struct Side {
    p0: Pt,
    c1: Pt,
    c2: Pt,
    m1: Pt,
    m2: Pt,
    c3: Pt,
    c4: Pt,
    p3: Pt,
}

/// Notch style: the reference profile. `x` is the inflection.
fn profile_side(x: f64, top: f64, bottom: f64, r: f64, c: f64, dir: f64) -> Side {
    let d = (bottom - top).max(0.0);
    let natural = (r + c) * REF_ASPECT;
    let curve_depth = d.min(natural);
    let gap = d - curve_depth; // straight wall, only when deeper than the reference shape
    let vr = curve_depth * REF_SPLIT;
    let vc = curve_depth - vr;
    let mut tx = r;
    let mut ty = 2.0 * vr + gap * 20.0;
    let len = tx.hypot(ty);
    let len = if len == 0.0 { 1.0 } else { len };
    tx = dir * tx / len;
    ty /= len;
    let hx = if ty > 1e-6 { gap * tx.abs() / ty } else { 0.0 };
    let h1 = H_FLARE_START * r;
    let h2 = H_FLARE_END * r.hypot(vr);
    let h3 = H_ROUND_START * c.hypot(vc);
    let h4 = H_ROUND_END * c;
    let x0 = x - dir * r;
    let x3 = x + dir * c;
    let m1 = Pt(x - dir * hx / 2.0, top + vr);
    let m2 = Pt(x + dir * hx / 2.0, bottom - vc);
    Side {
        p0: Pt(x0, top),
        c1: Pt(x0 + dir * h1, top),
        c2: Pt(m1.0 - tx * h2, m1.1 - ty * h2),
        m1,
        m2,
        c3: Pt(m2.0 + tx * h3, m2.1 + ty * h3),
        c4: Pt(x3 - dir * h4, bottom),
        p3: Pt(x3, bottom),
    }
}

/// Popup style: circular concave fillet, straight wall at `x`, circular corner.
fn rounded_side(x: f64, top: f64, bottom: f64, r: f64, c: f64, dir: f64) -> Side {
    Side {
        p0: Pt(x - dir * r, top),
        c1: Pt(x - dir * r * (1.0 - K), top),
        c2: Pt(x, top + r - r * K),
        m1: Pt(x, top + r),
        m2: Pt(x, bottom - c),
        c3: Pt(x, bottom - c + c * K),
        c4: Pt(x + dir * c * (1.0 - K), bottom),
        p3: Pt(x + dir * c, bottom),
    }
}

/// Shrinks rounded-style radii so both fit a bump of the given depth and width.
fn fit(r: f64, c: f64, depth: f64, width: f64) -> (f64, f64) {
    let d = depth.max(0.0);
    let c = c.min(width.max(0.0) / 2.0);
    if r + c > d {
        let k = if r + c > 0.0 { d / (r + c) } else { 0.0 };
        (r * k, c * k)
    } else {
        (r, c)
    }
}

fn mix(a: Side, b: Side, w: f64) -> Side {
    if w <= 0.0 {
        return a;
    }
    if w >= 1.0 {
        return b;
    }
    let m = |p: Pt, q: Pt| Pt(lerp(p.0, q.0, w), lerp(p.1, q.1, w));
    Side { p0: m(a.p0, b.p0), c1: m(a.c1, b.c1), c2: m(a.c2, b.c2), m1: m(a.m1, b.m1), m2: m(a.m2, b.m2), c3: m(a.c3, b.c3), c4: m(a.c4, b.c4), p3: m(a.p3, b.p3) }
}

fn emit_left(p: &mut Vec<Cmd>, s: &Side) {
    p.push(Cmd::L(s.p0));
    p.push(Cmd::C(s.c1, s.c2, s.m1));
    p.push(Cmd::L(s.m2));
    p.push(Cmd::C(s.c3, s.c4, s.p3));
}

fn emit_right(p: &mut Vec<Cmd>, s: &Side) {
    p.push(Cmd::L(s.p3));
    p.push(Cmd::C(s.c4, s.c3, s.m2));
    p.push(Cmd::L(s.m1));
    p.push(Cmd::C(s.c2, s.c1, s.p0));
}

/// Both sides of a bump; `w` blends the notch profile (0) into the rounded popup (1).
/// A side against the screen edge (a panel pushed into a corner) is straight, square.
#[allow(clippy::too_many_arguments)]
fn bump(p: &mut Vec<Cmd>, g: &Geometry, left: f64, right: f64, top: f64, bottom: f64, w: f64, k: f64, flush: (bool, bool)) {
    let nc = (g.notch_corner_radius * k).min((right - left).max(0.0) / 2.0);
    let cr = g.concave_radius * k;
    let (pr, pc) = fit(g.panel_concave_radius, g.panel_corner_radius, bottom - top, right - left);
    let side = |x: f64, dir: f64, square: bool| {
        if square {
            rounded_side(x, top, bottom, 0.0, 0.0, dir)
        } else {
            mix(profile_side(x, top, bottom, cr, nc, dir), rounded_side(x, top, bottom, pr, pc, dir), w)
        }
    };
    emit_left(p, &side(left, 1.0, flush.0));
    emit_right(p, &side(right, -1.0, flush.1));
}

/// `t`: expand progress (0 collapsed, 1 open; may overshoot); `s`: spread (0 the
/// hull spans the notches, 1 the screen); bumps left to right. `corner_radius`:
/// rounded screen corners drawn with the shape (0: off); `combine`: merge notches
/// that would touch.
pub fn compute(g: &Geometry, w: f64, corner_radius: f64, combine: bool, t: f64, s: f64, bumps: &[Bump]) -> Frame {
    let h = g.expanded_height;
    let r = g.concave_radius;
    let t0 = t.max(0.0);
    let base = h * t0;

    let frames: Vec<BumpFrame> = bumps
        .iter()
        .map(|b| {
            let p = b.p.max(0.0);
            let row_depth = lerp(b.height, h, t0);
            let depth = lerp(row_depth, b.panel_top.unwrap_or(h) + b.panel_height, p);
            let anchor_cx = b.x + b.width / 2.0;
            if let Some(step) = b.step {
                let pl = if step == Step::Left { 0.0 } else { w - b.panel_width };
                let pr = pl + b.panel_width;
                return BumpFrame { cx: (pl + pr) / 2.0, anchor_cx, left: b.x, right: b.x + b.width, row_depth, depth, panel: Some((pl, pr)), panel_x: pl };
            }
            // Centered on its notch but kept on screen; each side morphs toward the panel's.
            let panel_left = (anchor_cx - b.panel_width / 2.0).clamp(0.0, (w - b.panel_width).max(0.0));
            let mut left = lerp(anchor_cx - b.width / 2.0, panel_left, p);
            let mut right = lerp(anchor_cx + b.width / 2.0, panel_left + b.panel_width, p);
            // A panel that ends up flush with a screen edge grows out of it from the start.
            if p > 0.001 && b.panel_width < w {
                if anchor_cx - b.panel_width / 2.0 < 0.5 {
                    left = left.min(0.0);
                }
                if anchor_cx + b.panel_width / 2.0 > w - 0.5 {
                    right = right.max(w);
                }
            }
            BumpFrame { cx: (left + right) / 2.0, anchor_cx, left, right, row_depth, depth, panel: None, panel_x: panel_left }
        })
        .collect();

    // Hull ends: collapsed they hug the outermost notches, fully spread the screen edges.
    let pad = r + g.corner_radius + 1.0;
    let hull: Vec<&Bump> = bumps.iter().filter(|b| b.in_hull).collect();
    let (hl0, hr0) = if hull.is_empty() {
        (w / 2.0, w / 2.0)
    } else {
        (
            hull.iter().map(|b| b.x).fold(f64::INFINITY, f64::min) - pad,
            hull.iter().map(|b| b.x + b.width).fold(f64::NEG_INFINITY, f64::max) + pad,
        )
    };
    let hl = lerp(hl0, hl0.min(0.0), s);
    let hr = lerp(hr0, hr0.max(w), s);
    let (hr_, hc) = fit(g.panel_concave_radius, g.corner_radius * (1.0 - s).clamp(0.0, 1.0), base, hr - hl);

    let mut p = vec![Cmd::M(Pt(-OFF, 0.0))];
    let mut extra: Vec<Cmd> = Vec::new();
    emit_left(&mut p, &rounded_side(hl, 0.0, base, hr_, hc, 1.0));
    let style = |i: usize| bumps[i].rounded.unwrap_or(if bumps[i].in_hull { bumps[i].p.clamp(0.0, 1.0) } else { 1.0 });
    let invisible = |i: usize| frames[i].right - frames[i].left < 1.0 && frames[i].depth - base < 0.5;
    let plain = |i: usize| bumps[i].in_hull && bumps[i].p <= 0.001;
    const MERGE_GAP: f64 = 6.0;
    let mut i = 0;
    while i < frames.len() {
        if invisible(i) {
            i += 1;
            continue;
        }
        let (left, mut right, mut depth) = (frames[i].left, frames[i].right, frames[i].depth);
        let mut k = bumps[i].scale;
        if let (Some(_), Some((sl, sr))) = (bumps[i].step, frames[i].panel) {
            // The notch as it is; the panel stands on its row, with fillets where they meet.
            bump(&mut p, g, left, right, base, frames[i].row_depth, 0.0, k, (false, false));
            if bumps[i].p > 0.001 && depth - frames[i].row_depth > 0.5 {
                let mut q = Vec::new();
                bump(&mut q, g, sl, sr, frames[i].row_depth - 1.0, depth, 1.0, 1.0, (sl <= 0.5, sr >= w - 0.5));
                if let Some(Cmd::L(first)) = q.first().copied() {
                    q[0] = Cmd::M(first);
                }
                extra.extend(q);
                extra.push(Cmd::Z);
            }
            i += 1;
            continue;
        }
        let mut next = i + 1;
        if combine && plain(i) {
            // Notches that would touch or overlap become one shape; their widgets stay put.
            for j in i + 1..frames.len() {
                if invisible(j) {
                    continue;
                }
                if !plain(j) || frames[j].left - right - 2.0 * r * k > MERGE_GAP {
                    break;
                }
                right = right.max(frames[j].right);
                k = k.max(bumps[j].scale);
                depth = depth.max(frames[j].depth);
                next = j + 1;
            }
        }
        // Flush with the screen edge: only popups (a docked notch's own sides are off-screen anyway).
        let popup = style(i) > 0.001;
        bump(&mut p, g, left, right, base, depth, style(i), k, (popup && left <= 0.5, popup && right >= w - 0.5));
        i = next;
    }
    emit_right(&mut p, &rounded_side(hr, 0.0, base, hr_, hc, -1.0));
    p.push(Cmd::L(Pt(w + OFF, 0.0)));
    p.push(Cmd::L(Pt(w + OFF, -OFF)));
    p.push(Cmd::L(Pt(-OFF, -OFF)));
    p.push(Cmd::Z);
    p.extend(extra);

    // Rounded screen corners: a quarter-circle fillet hanging from whatever fills
    // that corner right now (the open bar, a notch hugging it, a panel).
    let rr = corner_radius;
    let mut corners = Vec::new();
    if rr > 0.0 {
        let depth_at = |left: bool| {
            let mut d = if (left && hl <= 0.5) || (!left && hr >= w - 0.5) { base } else { 0.0 };
            for f in &frames {
                if f.right - f.left < 1.0 {
                    continue;
                }
                if (left && f.left <= 1.0) || (!left && f.right >= w - 1.0) {
                    d = d.max(f.depth);
                }
            }
            d
        };
        let dl = depth_at(true);
        let dr = depth_at(false);
        // SVG "A R R 0 0 1" from the edge to R along it, as one cubic arc. Each reaches
        // 1 px into what it hangs from, so no anti-aliased seam shows between them.
        corners.push(Cmd::M(Pt(0.0, dl - 1.0)));
        corners.push(Cmd::L(Pt(0.0, dl + rr)));
        corners.push(Cmd::C(Pt(0.0, dl + rr - K * rr), Pt(rr - K * rr, dl), Pt(rr, dl)));
        corners.push(Cmd::L(Pt(rr, dl - 1.0)));
        corners.push(Cmd::Z);
        corners.push(Cmd::M(Pt(w, dr - 1.0)));
        corners.push(Cmd::L(Pt(w - rr, dr - 1.0)));
        corners.push(Cmd::L(Pt(w - rr, dr)));
        corners.push(Cmd::C(Pt(w - rr + K * rr, dr), Pt(w, dr + rr - K * rr), Pt(w, dr + rr)));
        corners.push(Cmd::Z);
    }

    Frame { path: p, corners, base, bumps: frames }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notch(x: f64, width: f64) -> Bump {
        Bump { x, width, height: 30.0, in_hull: true, ..Default::default() }
    }

    #[test]
    fn closed_notch_spans_its_flares() {
        let g = Geometry::default();
        let f = compute(&g, 1000.0, 0.0, true, 0.0, 1.0, &[notch(450.0, 100.0)]);
        assert_eq!(f.base, 0.0);
        assert_eq!(f.bumps[0].row_depth, 30.0);
        assert_eq!((f.bumps[0].left, f.bumps[0].right), (450.0, 550.0));
        assert_eq!(f.path.first(), Some(&Cmd::M(Pt(-OFF, 0.0))));
        assert_eq!(f.path.last(), Some(&Cmd::Z));
    }

    #[test]
    fn touching_notches_merge() {
        let g = Geometry::default();
        let apart = compute(&g, 1000.0, 0.0, true, 0.0, 1.0, &[notch(100.0, 100.0), notch(600.0, 100.0)]);
        let close = compute(&g, 1000.0, 0.0, true, 0.0, 1.0, &[notch(100.0, 100.0), notch(210.0, 100.0)]);
        // Each bump is 8 commands; merged ones draw one bump.
        assert_eq!(apart.path.len() - close.path.len(), 8);
    }

    #[test]
    fn panel_stays_on_screen() {
        let g = Geometry::default();
        let b = Bump { x: 10.0, width: 0.0, panel_width: 200.0, panel_height: 100.0, p: 1.0, ..Default::default() };
        let f = compute(&g, 1000.0, 0.0, true, 1.0, 1.0, &[b]);
        assert_eq!(f.bumps[0].panel_x, 0.0);
        assert_eq!(f.bumps[0].depth, g.expanded_height + 100.0);
    }
}
