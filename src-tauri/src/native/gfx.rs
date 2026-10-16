//! Drawing for the native surfaces: one Direct2D software render target bound to
//! each surface's bitmap in turn (a graphics-card device costs tens of MB; bars
//! this small draw in microseconds on the CPU), DirectWrite text with Kasumi's
//! bundled Inter (the same 600-weight files the pages load, one per alphabet,
//! stitched together with a font fallback) and Segoe Fluent Icons.
//!
//! Units are logical px: the render target's DPI is set to the monitor's.

use std::cell::RefCell;
use std::collections::HashMap;

use windows::core::{Interface, Result, HSTRING, PCWSTR};
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_BEZIER_SEGMENT, D2D1_COLOR_F, D2D1_FIGURE_BEGIN_FILLED, D2D1_FIGURE_BEGIN_HOLLOW, D2D1_FIGURE_END_CLOSED,
    D2D1_FIGURE_END_OPEN, D2D1_FILL_MODE_WINDING, D2D1_PIXEL_FORMAT, D2D_RECT_F, D2D_SIZE_F, D2D_SIZE_U,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, ID2D1Bitmap, ID2D1DCRenderTarget, ID2D1Factory, ID2D1PathGeometry, ID2D1SolidColorBrush, D2D1_BITMAP_PROPERTIES,
    D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_FEATURE_LEVEL_DEFAULT, D2D1_RENDER_TARGET_PROPERTIES,
    D2D1_RENDER_TARGET_TYPE_SOFTWARE, D2D1_RENDER_TARGET_USAGE_NONE, D2D1_ROUNDED_RECT, D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE,
};
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::HDC;
use windows_numerics::{Matrix3x2, Vector2};

use super::geometry::{Cmd, Pt};

pub type Color = D2D1_COLOR_F;

pub fn rgba(r: f32, g: f32, b: f32, a: f32) -> Color {
    Color { r, g, b, a }
}

pub fn white(a: f32) -> Color {
    rgba(1.0, 1.0, 1.0, a)
}

/// "#rrggbb" (or "rrggbb") as a colour.
pub fn hex(s: &str) -> Option<Color> {
    let s = s.trim().trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let n = u32::from_str_radix(s, 16).ok()?;
    Some(rgba(((n >> 16) & 0xff) as f32 / 255.0, ((n >> 8) & 0xff) as f32 / 255.0, (n & 0xff) as f32 / 255.0, 1.0))
}

pub fn with_alpha(c: Color, a: f32) -> Color {
    Color { a: c.a * a, ..c }
}

pub fn rect(x: f32, y: f32, w: f32, h: f32) -> D2D_RECT_F {
    D2D_RECT_F { left: x, top: y, right: x + w, bottom: y + h }
}

/// Which font a piece of text uses.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Family {
    /// Kasumi's own (the bundled Inter).
    Inter,
    /// An installed family ([theme] font / font_secondary, a widget's own).
    Named(String),
    /// Segoe Fluent Icons (Segoe MDL2 Assets on Windows 10).
    Icons,
}

impl Family {
    /// A configured family name: empty means Kasumi's own.
    pub fn from_config(name: &str) -> Family {
        let n = name.trim();
        if n.is_empty() || n.eq_ignore_ascii_case("inter") {
            Family::Inter
        } else {
            Family::Named(n.to_string())
        }
    }
}

/// A font: family, size (logical px) and weight (CSS numbers: 400, 600...).
#[derive(Clone, Debug, PartialEq)]
pub struct Font {
    pub family: Family,
    pub size: f32,
    pub weight: u32,
}

impl Font {
    pub fn new(family: Family, size: f32) -> Font {
        let weight = if family == Family::Icons { 400 } else { 600 };
        Font { family, size, weight }
    }
    pub fn icons(size: f32) -> Font {
        Font::new(Family::Icons, size)
    }
}

/// A laid-out line of text and its size.
pub struct Text {
    pub layout: IDWriteTextLayout,
    pub width: f32,
    pub height: f32,
}

type Ranges = &'static [(u32, u32)];

const INTER: [(&[u8], Ranges); 7] = [
    // Latin first: it's the format's own collection; the rest are its fallback.
    (
        include_bytes!("fonts/inter-latin-600-normal.woff2"),
        &[(0x0000, 0x00FF), (0x0131, 0x0131), (0x0152, 0x0153), (0x02BB, 0x02BC), (0x02C6, 0x02C6), (0x02DA, 0x02DA), (0x02DC, 0x02DC), (0x0304, 0x0304), (0x0308, 0x0308), (0x0329, 0x0329), (0x2000, 0x206F), (0x20AC, 0x20AC), (0x2122, 0x2122), (0x2191, 0x2191), (0x2193, 0x2193), (0x2212, 0x2212), (0x2215, 0x2215), (0xFEFF, 0xFEFF), (0xFFFD, 0xFFFD)],
    ),
    (
        include_bytes!("fonts/inter-latin-ext-600-normal.woff2"),
        &[(0x0100, 0x02BA), (0x02BD, 0x02C5), (0x02C7, 0x02CC), (0x02CE, 0x02D7), (0x02DD, 0x02FF), (0x0304, 0x0304), (0x0308, 0x0308), (0x0329, 0x0329), (0x1D00, 0x1DBF), (0x1E00, 0x1E9F), (0x1EF2, 0x1EFF), (0x2020, 0x2020), (0x20A0, 0x20AB), (0x20AD, 0x20C0), (0x2113, 0x2113), (0x2C60, 0x2C7F), (0xA720, 0xA7FF)],
    ),
    (
        include_bytes!("fonts/inter-vietnamese-600-normal.woff2"),
        &[(0x0102, 0x0103), (0x0110, 0x0111), (0x0128, 0x0129), (0x0168, 0x0169), (0x01A0, 0x01A1), (0x01AF, 0x01B0), (0x0300, 0x0301), (0x0303, 0x0304), (0x0308, 0x0309), (0x0323, 0x0323), (0x0329, 0x0329), (0x1EA0, 0x1EF9), (0x20AB, 0x20AB)],
    ),
    (include_bytes!("fonts/inter-cyrillic-600-normal.woff2"), &[(0x0301, 0x0301), (0x0400, 0x045F), (0x0490, 0x0491), (0x04B0, 0x04B1), (0x2116, 0x2116)]),
    (
        include_bytes!("fonts/inter-cyrillic-ext-600-normal.woff2"),
        &[(0x0460, 0x052F), (0x1C80, 0x1C8A), (0x20B4, 0x20B4), (0x2DE0, 0x2DFF), (0xA640, 0xA69F), (0xFE2E, 0xFE2F)],
    ),
    (include_bytes!("fonts/inter-greek-600-normal.woff2"), &[(0x0370, 0x0377), (0x037A, 0x037F), (0x0384, 0x038A), (0x038C, 0x038C), (0x038E, 0x03A1), (0x03A3, 0x03FF)]),
    (include_bytes!("fonts/inter-greek-ext-600-normal.woff2"), &[(0x1F00, 0x1FFF)]),
];

const fn tag(s: &[u8; 4]) -> u32 {
    s[0] as u32 | (s[1] as u32) << 8 | (s[2] as u32) << 16 | (s[3] as u32) << 24
}

/// The bundled Inter: the Latin collection and a fallback covering the other alphabets.
fn load_inter(dw: &IDWriteFactory5) -> Result<(IDWriteFontCollection1, IDWriteFontFallback)> {
    unsafe {
        let loader = dw.CreateInMemoryFontFileLoader()?;
        dw.RegisterFontFileLoader(&loader)?;
        let fallback = dw.CreateFontFallbackBuilder()?;
        let mut first: Option<IDWriteFontCollection1> = None;
        for (data, ranges) in INTER {
            let stream = dw.UnpackFontFile(DWRITE_CONTAINER_TYPE_WOFF2, data.as_ptr() as *const _, data.len() as u32)?;
            let size = stream.GetFileSize()?;
            let mut start = std::ptr::null_mut();
            let mut ctx = std::ptr::null_mut();
            stream.ReadFileFragment(&mut start, 0, size, &mut ctx)?;
            let bytes = std::slice::from_raw_parts(start as *const u8, size as usize).to_vec();
            stream.ReleaseFileFragment(ctx);
            // No owner: the loader keeps its own copy.
            let file = loader.CreateInMemoryFontFileReference(dw, bytes.as_ptr() as *const _, bytes.len() as u32, None)?;
            let builder = dw.CreateFontSetBuilder()?;
            builder.AddFontFile(&file)?;
            let collection = dw.CreateFontCollectionFromFontSet(&builder.CreateFontSet()?)?;
            let ranges: Vec<DWRITE_UNICODE_RANGE> = ranges.iter().map(|&(first, last)| DWRITE_UNICODE_RANGE { first, last }).collect();
            let name = HSTRING::from("Inter");
            fallback.AddMapping(&ranges, &[name.as_ptr()], &collection, PCWSTR::null(), PCWSTR::null(), 1.0)?;
            first.get_or_insert(collection);
        }
        fallback.AddMappings(&dw.GetSystemFontFallback()?)?;
        Ok((first.expect("Inter latin"), fallback.CreateFontFallback()?))
    }
}

pub struct Gfx {
    pub d2d: ID2D1Factory,
    pub dw: IDWriteFactory5,
    pub rt: ID2D1DCRenderTarget,
    pub brush: ID2D1SolidColorBrush,
    inter: Option<(IDWriteFontCollection1, IDWriteFontFallback)>,
    /// Tabular figures and Inter's open digits, as the pages ask for (font-feature-settings).
    typography: IDWriteTypography,
    formats: RefCell<HashMap<(Family, u32, u32), IDWriteTextFormat>>,
    /// Text layouts by (text, font): widgets draw the same strings every frame.
    texts: RefCell<HashMap<(String, Family, u32, u32), (IDWriteTextLayout, f32, f32)>>,
    /// SVG icon geometries, by their path string's address.
    svgs: RefCell<HashMap<usize, ID2D1PathGeometry>>,
    /// Round line ends (ring meters).
    round_caps: windows::Win32::Graphics::Direct2D::ID2D1StrokeStyle,
    /// The scale of the surface being drawn (device px per logical px).
    scale: std::cell::Cell<f32>,
}

impl Gfx {
    pub fn new() -> Result<Gfx> {
        unsafe {
            let d2d: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let dw: IDWriteFactory5 = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let props = D2D1_RENDER_TARGET_PROPERTIES {
                r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
                pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
                dpiX: 96.0,
                dpiY: 96.0,
                usage: D2D1_RENDER_TARGET_USAGE_NONE,
                minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
            };
            let rt = d2d.CreateDCRenderTarget(&props)?;
            // Grayscale text: ClearType needs an opaque background.
            rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            let brush = rt.CreateSolidColorBrush(&white(1.0), None)?;
            let inter = match load_inter(&dw) {
                Ok(v) => Some(v),
                Err(e) => {
                    eprintln!("[kasumi] native: bundled Inter not loaded ({e}); using Segoe UI");
                    None
                }
            };
            let typography = dw.CreateTypography()?;
            typography.AddFontFeature(DWRITE_FONT_FEATURE { nameTag: DWRITE_FONT_FEATURE_TAG(tag(b"tnum")), parameter: 1 })?;
            typography.AddFontFeature(DWRITE_FONT_FEATURE { nameTag: DWRITE_FONT_FEATURE_TAG(tag(b"cv11")), parameter: 1 })?;
            let round_caps = d2d.CreateStrokeStyle(
                &windows::Win32::Graphics::Direct2D::D2D1_STROKE_STYLE_PROPERTIES {
                    startCap: windows::Win32::Graphics::Direct2D::D2D1_CAP_STYLE_ROUND,
                    endCap: windows::Win32::Graphics::Direct2D::D2D1_CAP_STYLE_ROUND,
                    ..Default::default()
                },
                None,
            )?;
            Ok(Gfx {
                d2d,
                dw,
                rt,
                brush,
                inter,
                typography,
                formats: RefCell::new(HashMap::new()),
                texts: RefCell::new(HashMap::new()),
                svgs: RefCell::new(HashMap::new()),
                round_caps,
                scale: std::cell::Cell::new(1.0),
            })
        }
    }

    fn format(&self, font: &Font) -> Result<IDWriteTextFormat> {
        let key = (font.family.clone(), (font.size * 100.0) as u32, font.weight);
        if let Some(f) = self.formats.borrow().get(&key) {
            return Ok(f.clone());
        }
        let weight = DWRITE_FONT_WEIGHT(font.weight as i32);
        let make = |name: &str, collection: Option<&IDWriteFontCollection>| unsafe {
            self.dw.CreateTextFormat(&HSTRING::from(name), collection, weight, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, font.size, &HSTRING::from("en-us"))
        };
        let f: IDWriteTextFormat = match (&font.family, self.inter.as_ref()) {
            (Family::Inter, Some((collection, fallback))) => {
                let f = make("Inter", Some(&collection.cast()?))?;
                unsafe { f.cast::<IDWriteTextFormat1>()?.SetFontFallback(fallback)? };
                f
            }
            (Family::Inter, None) => make("Segoe UI", None)?,
            (Family::Icons, _) => make(if has_family(&self.dw, "Segoe Fluent Icons") { "Segoe Fluent Icons" } else { "Segoe MDL2 Assets" }, None)?,
            (Family::Named(name), _) => make(name, None)?,
        };
        unsafe { f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)? };
        self.formats.borrow_mut().insert(key, f.clone());
        Ok(f)
    }

    /// One line of text, laid out (cached: widgets draw the same strings every frame).
    pub fn text(&self, s: &str, font: &Font) -> Result<Text> {
        let key = (s.to_string(), font.family.clone(), (font.size * 100.0) as u32, font.weight);
        if let Some((layout, width, height)) = self.texts.borrow().get(&key) {
            return Ok(Text { layout: layout.clone(), width: *width, height: *height });
        }
        let wide: Vec<u16> = s.encode_utf16().collect();
        let layout = unsafe { self.dw.CreateTextLayout(&wide, &self.format(font)?, 100_000.0, 1000.0)? };
        if font.family != Family::Icons {
            unsafe { layout.SetTypography(&self.typography, DWRITE_TEXT_RANGE { startPosition: 0, length: wide.len() as u32 })? };
        }
        let mut m = DWRITE_TEXT_METRICS::default();
        unsafe { layout.GetMetrics(&mut m)? };
        let (width, height) = (m.widthIncludingTrailingWhitespace, m.height);
        let mut texts = self.texts.borrow_mut();
        if texts.len() > 600 {
            texts.clear();
        }
        texts.insert(key, (layout.clone(), width, height));
        Ok(Text { layout, width, height })
    }

    /// Width of a string in a font (0 if it can't be laid out).
    pub fn measure(&self, s: &str, font: &Font) -> f32 {
        self.text(s, font).map_or(0.0, |t| t.width)
    }

    /// Draws text with its top-left at (x, y).
    pub fn draw_text(&self, t: &Text, x: f32, y: f32, c: Color) {
        unsafe {
            self.brush.SetColor(&c);
            self.rt.DrawTextLayout(Vector2 { X: x, Y: y }, &t.layout, &self.brush, D2D1_DRAW_TEXT_OPTIONS_NONE);
        }
    }

    /// Draws text vertically centred on `cy`, starting at x. Returns its width.
    pub fn text_at(&self, s: &str, font: &Font, x: f32, cy: f32, c: Color) -> f32 {
        let Ok(t) = self.text(s, font) else { return 0.0 };
        // Whole device pixels: text at fractional offsets shimmers as things move.
        self.draw_text(&t, self.snap(x), self.snap(cy - t.height / 2.0), c);
        t.width
    }

    /// The scale of the surface being drawn.
    pub fn scale(&self) -> f32 {
        self.scale.get()
    }

    /// A logical coordinate on a whole device pixel.
    pub fn snap(&self, v: f32) -> f32 {
        let k = self.scale.get();
        (v * k).round() / k
    }

    pub fn fill_rect(&self, r: D2D_RECT_F, c: Color) {
        unsafe {
            self.brush.SetColor(&c);
            self.rt.FillRectangle(&r, &self.brush);
        }
    }

    pub fn fill_round(&self, r: D2D_RECT_F, radius: f32, c: Color) {
        unsafe {
            self.brush.SetColor(&c);
            self.rt.FillRoundedRectangle(&D2D1_ROUNDED_RECT { rect: r, radiusX: radius, radiusY: radius }, &self.brush);
        }
    }

    /// Path commands (logical px from the surface's edge) as a geometry in window
    /// space: `flip` = a bottom surface, y measured up from that height.
    pub fn path(&self, cmds: &[Cmd], flip: Option<f32>) -> Result<ID2D1PathGeometry> {
        let map = |p: Pt| match flip {
            Some(h) => Vector2 { X: p.0 as f32, Y: h - p.1 as f32 },
            None => Vector2 { X: p.0 as f32, Y: p.1 as f32 },
        };
        unsafe {
            let geom = self.d2d.CreatePathGeometry()?;
            let sink = geom.Open()?;
            sink.SetFillMode(D2D1_FILL_MODE_WINDING);
            let mut open = false;
            for c in cmds {
                match c {
                    Cmd::M(p) => {
                        if open {
                            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
                        }
                        sink.BeginFigure(map(*p), D2D1_FIGURE_BEGIN_FILLED);
                        open = true;
                    }
                    Cmd::L(p) => sink.AddLine(map(*p)),
                    Cmd::C(a, b, p) => sink.AddBezier(&D2D1_BEZIER_SEGMENT { point1: map(*a), point2: map(*b), point3: map(*p) }),
                    Cmd::Z => {
                        sink.EndFigure(D2D1_FIGURE_END_CLOSED);
                        open = false;
                    }
                }
            }
            if open {
                sink.EndFigure(D2D1_FIGURE_END_CLOSED);
            }
            sink.Close()?;
            Ok(geom)
        }
    }

    /// An icon's straight RGBA pixels as a bitmap for the render target.
    pub fn bitmap(&self, w: u32, h: u32, rgba: &[u8]) -> Result<ID2D1Bitmap> {
        let mut px = Vec::with_capacity(rgba.len());
        for p in rgba.chunks_exact(4) {
            let a = p[3] as u32;
            px.extend_from_slice(&[(p[2] as u32 * a / 255) as u8, (p[1] as u32 * a / 255) as u8, (p[0] as u32 * a / 255) as u8, p[3]]);
        }
        let props = D2D1_BITMAP_PROPERTIES {
            pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
            dpiX: 96.0,
            dpiY: 96.0,
        };
        unsafe { self.rt.CreateBitmap(D2D_SIZE_U { width: w, height: h }, Some(px.as_ptr() as *const _), w * 4, &props) }
    }

    /// An SVG path's `d` (absolute and relative M L H V C S Q T A Z) as a geometry,
    /// cached (icons are drawn every frame).
    pub fn svg(&self, d: &'static str) -> Result<ID2D1PathGeometry> {
        if let Some(g) = self.svgs.borrow().get(&(d.as_ptr() as usize)) {
            return Ok(g.clone());
        }
        let g = svg_path(&self.d2d, d)?;
        self.svgs.borrow_mut().insert(d.as_ptr() as usize, g.clone());
        Ok(g)
    }

    /// Fills an SVG icon (`view` px square) `size` px wide with its top-left at (x, y).
    pub fn draw_svg(&self, d: &'static str, view: f32, x: f32, y: f32, size: f32, c: Color) {
        let Ok(g) = self.svg(d) else { return };
        let k = size / view;
        unsafe {
            let mut was = Matrix3x2::identity();
            self.rt.GetTransform(&mut was);
            self.rt.SetTransform(&(Matrix3x2 { M11: k, M12: 0.0, M21: 0.0, M22: k, M31: x, M32: y } * was));
            self.brush.SetColor(&c);
            self.rt.FillGeometry(&g, &self.brush, None::<&windows::Win32::Graphics::Direct2D::ID2D1Brush>);
            self.rt.SetTransform(&was);
        }
    }

    /// A ring meter: a track circle and an arc from the top, clockwise, `v` of the way round.
    pub fn ring(&self, cx: f32, cy: f32, r: f32, width: f32, v: f32, track: Color, fill: Color) {
        use windows::Win32::Graphics::Direct2D::*;
        unsafe {
            self.brush.SetColor(&track);
            self.rt.DrawEllipse(&D2D1_ELLIPSE { point: Vector2 { X: cx, Y: cy }, radiusX: r, radiusY: r }, &self.brush, width, None);
            let v = v.clamp(0.0, 1.0);
            if v <= 0.0 {
                return;
            }
            let Ok(geom) = self.d2d.CreatePathGeometry() else { return };
            let Ok(sink) = geom.Open() else { return };
            let at = |a: f32| Vector2 { X: cx + r * a.sin(), Y: cy - r * a.cos() };
            let end = (v.min(0.9999)) * std::f32::consts::TAU;
            sink.BeginFigure(at(0.0), D2D1_FIGURE_BEGIN_HOLLOW);
            sink.AddArc(&D2D1_ARC_SEGMENT {
                point: at(end),
                size: D2D_SIZE_F { width: r, height: r },
                rotationAngle: 0.0,
                sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                arcSize: if v > 0.5 { D2D1_ARC_SIZE_LARGE } else { D2D1_ARC_SIZE_SMALL },
            });
            sink.EndFigure(D2D1_FIGURE_END_OPEN);
            let _ = sink.Close();
            self.brush.SetColor(&fill);
            self.rt.DrawGeometry(&geom, &self.brush, width, &self.round_caps);
        }
    }

    /// Clips what's drawn next to a rectangle whose ends fade out over `left` / `right`
    /// px (a scrolling title's CSS mask), until `pop_fade`.
    pub fn push_fade(&self, r: D2D_RECT_F, left: f32, right: f32) {
        use windows::Win32::Graphics::Direct2D::*;
        let w = (r.right - r.left).max(1.0);
        let stops = [
            windows::Win32::Graphics::Direct2D::Common::D2D1_GRADIENT_STOP { position: 0.0, color: rgba(0.0, 0.0, 0.0, 0.0) },
            windows::Win32::Graphics::Direct2D::Common::D2D1_GRADIENT_STOP { position: (left / w).clamp(0.0, 0.5), color: rgba(0.0, 0.0, 0.0, 1.0) },
            windows::Win32::Graphics::Direct2D::Common::D2D1_GRADIENT_STOP { position: (1.0 - right / w).clamp(0.5, 1.0), color: rgba(0.0, 0.0, 0.0, 1.0) },
            windows::Win32::Graphics::Direct2D::Common::D2D1_GRADIENT_STOP { position: 1.0, color: rgba(0.0, 0.0, 0.0, 0.0) },
        ];
        unsafe {
            let Ok(coll) = self.rt.CreateGradientStopCollection(&stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP) else { return };
            let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES { startPoint: Vector2 { X: r.left, Y: r.top }, endPoint: Vector2 { X: r.right, Y: r.top } };
            let Ok(brush) = self.rt.CreateLinearGradientBrush(&props, None, &coll) else { return };
            let params = D2D1_LAYER_PARAMETERS {
                contentBounds: r,
                geometricMask: std::mem::ManuallyDrop::new(None),
                maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                maskTransform: Matrix3x2::identity(),
                opacity: 1.0,
                opacityBrush: std::mem::ManuallyDrop::new(Some(brush.into())),
                layerOptions: D2D1_LAYER_OPTIONS_NONE,
            };
            self.rt.PushLayer(&params, None);
            std::mem::ManuallyDrop::into_inner(params.opacityBrush);
        }
    }

    pub fn pop_fade(&self) {
        unsafe { self.rt.PopLayer() }
    }

    /// Clips what's drawn next to a rectangle (until `unclip`).
    pub fn clip(&self, r: D2D_RECT_F) {
        unsafe { self.rt.PushAxisAlignedClip(&r, windows::Win32::Graphics::Direct2D::D2D1_ANTIALIAS_MODE_ALIASED) }
    }

    pub fn unclip(&self) {
        unsafe { self.rt.PopAxisAlignedClip() }
    }

    /// Starts drawing into a surface's bitmap (physical w × h) at its monitor's scale.
    pub fn begin(&self, dc: HDC, w: i32, h: i32, scale: f32) -> Result<()> {
        unsafe {
            self.rt.BindDC(dc, &RECT { left: 0, top: 0, right: w, bottom: h })?;
            self.rt.SetDpi(96.0 * scale, 96.0 * scale);
            self.scale.set(scale);
            self.rt.BeginDraw();
            self.rt.SetTransform(&Matrix3x2::identity());
            self.rt.Clear(Some(&rgba(0.0, 0.0, 0.0, 0.0)));
        }
        Ok(())
    }

    pub fn end(&self) -> Result<()> {
        unsafe { self.rt.EndDraw(None, None) }
    }
}

/// Parses an SVG path into a Direct2D geometry (nonzero fill, as SVG's default).
fn svg_path(d2d: &ID2D1Factory, d: &str) -> Result<ID2D1PathGeometry> {
    use windows::Win32::Graphics::Direct2D::*;
    let b = d.as_bytes();
    let mut i = 0;
    let skip = |i: &mut usize| {
        while *i < b.len() && (b[*i] == b' ' || b[*i] == b',' || b[*i] == b'\n' || b[*i] == b'\t' || b[*i] == b'\r') {
            *i += 1;
        }
    };
    let number = |i: &mut usize| -> Option<f32> {
        skip(i);
        let start = *i;
        if *i < b.len() && (b[*i] == b'-' || b[*i] == b'+') {
            *i += 1;
        }
        let mut dot = false;
        while *i < b.len() && (b[*i].is_ascii_digit() || (b[*i] == b'.' && !dot)) {
            dot |= b[*i] == b'.';
            *i += 1;
        }
        if *i < b.len() && (b[*i] == b'e' || b[*i] == b'E') {
            *i += 1;
            if *i < b.len() && (b[*i] == b'-' || b[*i] == b'+') {
                *i += 1;
            }
            while *i < b.len() && b[*i].is_ascii_digit() {
                *i += 1;
            }
        }
        std::str::from_utf8(&b[start..*i]).ok()?.parse().ok()
    };
    // Arc flags may be written without separators ("a1 1 0 01.5.5").
    let flag = |i: &mut usize| -> Option<bool> {
        skip(i);
        let f = *b.get(*i)?;
        *i += 1;
        Some(f == b'1')
    };
    let v = |x: f32, y: f32| Vector2 { X: x, Y: y };
    unsafe {
        let geom = d2d.CreatePathGeometry()?;
        let sink = geom.Open()?;
        sink.SetFillMode(D2D1_FILL_MODE_WINDING);
        let (mut cx, mut cy, mut sx, mut sy) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
        // The last control point (S / T reflect it) and which kind of curve set it.
        let (mut qx, mut qy, mut last) = (0.0f32, 0.0f32, b' ');
        let mut open = false;
        let mut cmd = b'M';
        loop {
            skip(&mut i);
            if i >= b.len() {
                break;
            }
            if b[i].is_ascii_alphabetic() {
                cmd = b[i];
                i += 1;
            }
            let rel = cmd.is_ascii_lowercase();
            let (ox, oy) = if rel { (cx, cy) } else { (0.0, 0.0) };
            let begin = |open: &mut bool, x: f32, y: f32| {
                if !*open {
                    sink.BeginFigure(v(x, y), D2D1_FIGURE_BEGIN_FILLED);
                    *open = true;
                }
            };
            match cmd.to_ascii_uppercase() {
                b'Z' => {
                    if open {
                        sink.EndFigure(D2D1_FIGURE_END_CLOSED);
                        open = false;
                    }
                    cx = sx;
                    cy = sy;
                    last = b'Z';
                    continue;
                }
                b'M' => {
                    let (Some(x), Some(y)) = (number(&mut i), number(&mut i)) else { break };
                    if open {
                        sink.EndFigure(D2D1_FIGURE_END_OPEN);
                        open = false;
                    }
                    cx = ox + x;
                    cy = oy + y;
                    sx = cx;
                    sy = cy;
                    begin(&mut open, cx, cy);
                    // Further pairs are line-tos.
                    cmd = if rel { b'l' } else { b'L' };
                }
                b'L' | b'H' | b'V' => {
                    let up = cmd.to_ascii_uppercase();
                    let (x, y) = match up {
                        b'H' => (number(&mut i).map(|x| ox + x), Some(cy)),
                        b'V' => (Some(cx), number(&mut i).map(|y| oy + y)),
                        _ => (number(&mut i).map(|x| ox + x), number(&mut i).map(|y| oy + y)),
                    };
                    let (Some(x), Some(y)) = (x, y) else { break };
                    begin(&mut open, cx, cy);
                    sink.AddLine(v(x, y));
                    cx = x;
                    cy = y;
                }
                b'C' | b'S' => {
                    let (x1, y1) = if cmd.to_ascii_uppercase() == b'S' {
                        if last == b'C' { (2.0 * cx - qx, 2.0 * cy - qy) } else { (cx, cy) }
                    } else {
                        let (Some(a), Some(b2)) = (number(&mut i), number(&mut i)) else { break };
                        (ox + a, oy + b2)
                    };
                    let (Some(x2), Some(y2), Some(x), Some(y)) = (number(&mut i), number(&mut i), number(&mut i), number(&mut i)) else { break };
                    let (x2, y2, x, y) = (ox + x2, oy + y2, ox + x, oy + y);
                    begin(&mut open, cx, cy);
                    sink.AddBezier(&D2D1_BEZIER_SEGMENT { point1: v(x1, y1), point2: v(x2, y2), point3: v(x, y) });
                    qx = x2;
                    qy = y2;
                    cx = x;
                    cy = y;
                    last = b'C';
                    continue;
                }
                b'Q' | b'T' => {
                    let (x1, y1) = if cmd.to_ascii_uppercase() == b'T' {
                        if last == b'Q' { (2.0 * cx - qx, 2.0 * cy - qy) } else { (cx, cy) }
                    } else {
                        let (Some(a), Some(b2)) = (number(&mut i), number(&mut i)) else { break };
                        (ox + a, oy + b2)
                    };
                    let (Some(x), Some(y)) = (number(&mut i), number(&mut i)) else { break };
                    let (x, y) = (ox + x, oy + y);
                    begin(&mut open, cx, cy);
                    sink.AddQuadraticBezier(&D2D1_QUADRATIC_BEZIER_SEGMENT { point1: v(x1, y1), point2: v(x, y) });
                    qx = x1;
                    qy = y1;
                    cx = x;
                    cy = y;
                    last = b'Q';
                    continue;
                }
                b'A' => {
                    let (Some(rx), Some(ry), Some(rot)) = (number(&mut i), number(&mut i), number(&mut i)) else { break };
                    let (Some(large), Some(sweep)) = (flag(&mut i), flag(&mut i)) else { break };
                    let (Some(x), Some(y)) = (number(&mut i), number(&mut i)) else { break };
                    let (x, y) = (ox + x, oy + y);
                    begin(&mut open, cx, cy);
                    sink.AddArc(&D2D1_ARC_SEGMENT {
                        point: v(x, y),
                        size: D2D_SIZE_F { width: rx.abs(), height: ry.abs() },
                        rotationAngle: rot,
                        sweepDirection: if sweep { D2D1_SWEEP_DIRECTION_CLOCKWISE } else { D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE },
                        arcSize: if large { D2D1_ARC_SIZE_LARGE } else { D2D1_ARC_SIZE_SMALL },
                    });
                    cx = x;
                    cy = y;
                }
                _ => break,
            }
            last = cmd.to_ascii_uppercase();
        }
        if open {
            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
        }
        sink.Close()?;
        Ok(geom)
    }
}

fn has_family(dw: &IDWriteFactory5, name: &str) -> bool {
    unsafe {
        let mut coll: Option<IDWriteFontCollection> = None;
        let Ok(base) = dw.cast::<IDWriteFactory>() else { return false };
        if base.GetSystemFontCollection(&mut coll, false).is_err() {
            return false;
        }
        let Some(coll) = coll else { return false };
        let mut index = 0u32;
        let mut exists = windows::core::BOOL(0);
        coll.FindFamilyName(&HSTRING::from(name), &mut index, &mut exists).is_ok() && exists.as_bool()
    }
}
