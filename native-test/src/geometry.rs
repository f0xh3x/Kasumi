//! A port of Kasumi's src/geometry.ts (`computeFrame`) for a bar's notches: the
//! same outline, curve for curve, so the native test has Kasumi's exact shapes.
//! (Panels and dropdowns aren't here yet: the notches only morph into the bar.)

pub const OFF: f32 = 64.0; // how far outside the window the closing edges run

// The reference notch's profile (Asset 1.svg), as in geometry.ts.
const REF_ASPECT: f32 = 40.17 / 78.33;
const REF_SPLIT: f32 = 19.92 / 40.17;
const H_FLARE_START: f32 = 24.33 / 44.25;
const H_FLARE_END: f32 = 8.47 / 48.53;
const H_ROUND_START: f32 = 13.91 / 39.64;
const H_ROUND_END: f32 = 15.66 / 34.08;
const K: f32 = 0.5523; // cubic Bézier circle-arc constant

#[derive(Clone, Copy, Debug)]
pub struct Pt(pub f32, pub f32);

#[derive(Debug)]
pub enum Cmd {
    M(Pt),
    L(Pt),
    C(Pt, Pt, Pt),
    Z,
}

/// [geometry] in px (already scaled for the monitor).
pub struct Geometry {
    pub expanded_height: f32,
    pub concave_radius: f32,
    pub notch_corner_radius: f32,
    pub panel_concave_radius: f32,
    /// The hull's bottom corners while it spreads ([geometry] corner_radius).
    pub corner_radius: f32,
    /// Rounded screen corners ([general] corner_radius; 0 = off).
    pub screen_corner: f32,
    pub combine: bool,
}

/// A notch: its collapsed left edge, width and depth.
pub struct Bump {
    pub x: f32,
    pub width: f32,
    pub height: f32,
}

pub struct Frame {
    pub path: Vec<Cmd>,
    pub corners: Vec<Cmd>,
    /// The full-width bar's depth.
    pub base: f32,
    /// Each notch's content row depth.
    pub rows: Vec<f32>,
}

fn lerp(a: f32, b: f32, k: f32) -> f32 {
    a + (b - a) * k
}

/// One side of a bump, from the edge above (p0) to the floor (p3).
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
fn profile_side(x: f32, top: f32, bottom: f32, r: f32, c: f32, dir: f32) -> Side {
    let d = (bottom - top).max(0.0);
    let natural = (r + c) * REF_ASPECT;
    let curve_depth = d.min(natural);
    let gap = d - curve_depth;
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
fn rounded_side(x: f32, top: f32, bottom: f32, r: f32, c: f32, dir: f32) -> Side {
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
fn fit(r: f32, c: f32, depth: f32, width: f32) -> (f32, f32) {
    let d = depth.max(0.0);
    let c = c.min(width.max(0.0) / 2.0);
    if r + c > d {
        let k = if r + c > 0.0 { d / (r + c) } else { 0.0 };
        (r * k, c * k)
    } else {
        (r, c)
    }
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

/// A notch's two sides (notch profile; panels would blend toward the rounded style).
fn bump(p: &mut Vec<Cmd>, g: &Geometry, left: f32, right: f32, top: f32, bottom: f32) {
    let nc = g.notch_corner_radius.min((right - left).max(0.0) / 2.0);
    let cr = g.concave_radius;
    emit_left(p, &profile_side(left, top, bottom, cr, nc, 1.0));
    emit_right(p, &profile_side(right, top, bottom, cr, nc, -1.0));
}

/// `t`: expand progress (0 collapsed, 1 open); `s`: spread (0 the hull spans the
/// notches, 1 the screen: Kasumi's "drop" style keeps it at 1).
pub fn compute(g: &Geometry, w: f32, t: f32, s: f32, bumps: &[Bump]) -> Frame {
    let h = g.expanded_height;
    let r = g.concave_radius;
    let t0 = t.max(0.0);
    let base = h * t0;
    let frames: Vec<(f32, f32, f32)> = bumps.iter().map(|b| (b.x, b.x + b.width, lerp(b.height, h, t0))).collect();

    let pad = r + g.corner_radius + 1.0;
    let hl0 = bumps.iter().map(|b| b.x).fold(f32::INFINITY, f32::min) - pad;
    let hr0 = bumps.iter().map(|b| b.x + b.width).fold(f32::NEG_INFINITY, f32::max) + pad;
    let (hl0, hr0) = if bumps.is_empty() { (w / 2.0, w / 2.0) } else { (hl0, hr0) };
    let hl = lerp(hl0, hl0.min(0.0), s);
    let hr = lerp(hr0, hr0.max(w), s);
    let (hr_, hc) = fit(g.panel_concave_radius, g.corner_radius * (1.0 - s).clamp(0.0, 1.0), base, hr - hl);

    let mut p = vec![Cmd::M(Pt(-OFF, 0.0))];
    emit_left(&mut p, &rounded_side(hl, 0.0, base, hr_, hc, 1.0));
    const MERGE_GAP: f32 = 6.0;
    let invisible = |i: usize| frames[i].1 - frames[i].0 < 1.0 && frames[i].2 - base < 0.5;
    let mut i = 0;
    while i < frames.len() {
        if invisible(i) {
            i += 1;
            continue;
        }
        let (left, mut right, mut depth) = frames[i];
        let mut next = i + 1;
        if g.combine {
            // Notches that would touch or overlap become one shape.
            for (j, f) in frames.iter().enumerate().skip(i + 1) {
                if invisible(j) {
                    continue;
                }
                if f.0 - right - 2.0 * r > MERGE_GAP {
                    break;
                }
                right = right.max(f.1);
                depth = depth.max(f.2);
                next = j + 1;
            }
        }
        bump(&mut p, g, left, right, base, depth);
        i = next;
    }
    emit_right(&mut p, &rounded_side(hr, 0.0, base, hr_, hc, -1.0));
    p.push(Cmd::L(Pt(w + OFF, 0.0)));
    p.push(Cmd::L(Pt(w + OFF, -OFF)));
    p.push(Cmd::L(Pt(-OFF, -OFF)));
    p.push(Cmd::Z);

    // Rounded screen corners: a quarter-circle fillet hanging from whatever fills
    // that corner (the open bar, a notch hugging it).
    let rr = g.screen_corner;
    let mut corners = Vec::new();
    if rr > 0.0 {
        let depth_at = |left: bool| {
            let mut d = if (left && hl <= 0.5) || (!left && hr >= w - 0.5) { base } else { 0.0 };
            for f in &frames {
                if f.1 - f.0 < 1.0 {
                    continue;
                }
                if (left && f.0 <= 1.0) || (!left && f.1 >= w - 1.0) {
                    d = d.max(f.2);
                }
            }
            d
        };
        let dl = depth_at(true);
        let dr = depth_at(false);
        // SVG "A R R 0 0 1": centre (R, dl + R) / (W − R, dr + R), as cubic arcs.
        corners.push(Cmd::M(Pt(0.0, dl)));
        corners.push(Cmd::L(Pt(0.0, dl + rr)));
        corners.push(Cmd::C(Pt(0.0, dl + rr - K * rr), Pt(rr - K * rr, dl), Pt(rr, dl)));
        corners.push(Cmd::Z);
        corners.push(Cmd::M(Pt(w, dr)));
        corners.push(Cmd::L(Pt(w - rr, dr)));
        corners.push(Cmd::C(Pt(w - rr + K * rr, dr), Pt(w, dr + rr - K * rr), Pt(w, dr + rr)));
        corners.push(Cmd::Z);
    }

    Frame { path: p, corners, base, rows: frames.iter().map(|f| f.2).collect() }
}
