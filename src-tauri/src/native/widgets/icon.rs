//! App icons as the providers send them ({ w, h, rgba: base64 straight RGBA },
//! 64 px for tasks), drawn small and sharp: halved copies (a mip chain) so a
//! 64 px icon shown at 18 px is filtered from 32 px, not skipped across. The
//! pages' icon styles are baked in once: colour, mono (greyscale), accent
//! (greyscale tinted with the accent, as CSS `mix-blend-mode: color`).

use base64::Engine as _;
use serde_json::Value;
use windows::Win32::Graphics::Direct2D::{ID2D1Bitmap, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR};

use crate::native::gfx::{rect, Color, Gfx};

/// How icons look ([widgets.<w>] icons).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Style {
    Color,
    Mono,
    Accent(Color),
}

impl Style {
    pub fn from(name: Option<&str>, accent: Color) -> Style {
        match name {
            Some("mono") => Style::Mono,
            Some("accent") => Style::Accent(accent),
            _ => Style::Color,
        }
    }
}

/// Decoded pixels: (width, height, straight RGBA).
pub type Pixels = (u32, u32, Vec<u8>);

/// An icon JSON value's pixels.
pub fn decode(v: &Value) -> Option<Pixels> {
    let w = v.get("w")?.as_u64()? as u32;
    let h = v.get("h")?.as_u64()? as u32;
    let rgba = base64::engine::general_purpose::STANDARD.decode(v.get("rgba")?.as_str()?).ok()?;
    (rgba.len() == (w * h * 4) as usize).then_some((w, h, rgba))
}

fn lum(r: f32, g: f32, b: f32) -> f32 {
    0.3 * r + 0.59 * g + 0.11 * b
}

/// The W3C blend modes' SetLum: colour `c` with luminosity `l`.
fn set_lum(c: (f32, f32, f32), l: f32) -> (f32, f32, f32) {
    let d = l - lum(c.0, c.1, c.2);
    let (r, g, b) = (c.0 + d, c.1 + d, c.2 + d);
    let l = lum(r, g, b);
    let n = r.min(g).min(b);
    let x = r.max(g).max(b);
    let clip = |v: f32| {
        let mut v = v;
        if n < 0.0 {
            v = l + (v - l) * l / (l - n).max(1e-6);
        }
        if x > 1.0 {
            v = l + (v - l) * (1.0 - l) / (x - l).max(1e-6);
        }
        v
    };
    (clip(r), clip(g), clip(b))
}

/// The style applied to the pixels.
fn styled(px: &[u8], style: Style) -> Vec<u8> {
    if style == Style::Color {
        return px.to_vec();
    }
    let mut out = px.to_vec();
    for p in out.chunks_exact_mut(4) {
        let (r, g, b) = (p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0);
        let grey = lum(r, g, b);
        let (r, g, b) = match style {
            Style::Mono => (grey, grey, grey),
            Style::Accent(a) => {
                // grayscale(1) contrast(1.1), then the accent's hue and saturation over it.
                let l = ((grey - 0.5) * 1.1 + 0.5).clamp(0.0, 1.0);
                set_lum((a.r, a.g, a.b), l)
            }
            Style::Color => (r, g, b),
        };
        p[0] = (r.clamp(0.0, 1.0) * 255.0).round() as u8;
        p[1] = (g.clamp(0.0, 1.0) * 255.0).round() as u8;
        p[2] = (b.clamp(0.0, 1.0) * 255.0).round() as u8;
    }
    out
}

/// Half the size, each pixel the (alpha-weighted) average of four.
fn halve(w: u32, h: u32, px: &[u8]) -> Pixels {
    let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
    let mut out = vec![0u8; (nw * nh * 4) as usize];
    for y in 0..nh {
        for x in 0..nw {
            let mut acc = [0u32; 4];
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let (sx, sy) = ((x * 2 + dx).min(w - 1), (y * 2 + dy).min(h - 1));
                let i = ((sy * w + sx) * 4) as usize;
                let a = px[i + 3] as u32;
                acc[0] += px[i] as u32 * a;
                acc[1] += px[i + 1] as u32 * a;
                acc[2] += px[i + 2] as u32 * a;
                acc[3] += a;
            }
            let o = ((y * nw + x) * 4) as usize;
            if acc[3] > 0 {
                out[o] = (acc[0] / acc[3]) as u8;
                out[o + 1] = (acc[1] / acc[3]) as u8;
                out[o + 2] = (acc[2] / acc[3]) as u8;
            }
            out[o + 3] = (acc[3] / 4) as u8;
        }
    }
    (nw, nh, out)
}

/// An icon ready to draw: bitmaps at full, half and quarter size.
pub struct AppIcon {
    mips: Vec<(u32, ID2D1Bitmap)>,
}

impl AppIcon {
    pub fn new(gfx: &Gfx, px: &Pixels, style: Style) -> Option<AppIcon> {
        let (mut w, mut h) = (px.0, px.1);
        let mut data = styled(&px.2, style);
        let mut mips = Vec::new();
        loop {
            mips.push((w, gfx.bitmap(w, h, &data).ok()?));
            if w <= 16 || mips.len() >= 3 {
                break;
            }
            let (nw, nh, nd) = halve(w, h, &data);
            (w, h, data) = (nw, nh, nd);
        }
        Some(AppIcon { mips })
    }

    /// Draws it `size` px square (logical) with its top-left at (x, y).
    pub fn draw(&self, gfx: &Gfx, x: f32, y: f32, size: f32, opacity: f32) {
        // The smallest copy still at least the size it's drawn at (in device px).
        let px = size * gfx.scale();
        let bmp = self.mips.iter().rev().find(|(w, _)| *w as f32 >= px).or(self.mips.first()).map(|(_, b)| b);
        if let Some(b) = bmp {
            unsafe {
                gfx.rt.DrawBitmap(b, Some(&rect(x, y, size, size)), opacity, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, None);
            }
        }
    }
}
