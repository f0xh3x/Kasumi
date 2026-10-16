//! One surface (a top bar or a dock on one monitor): the port of `Overlay` in
//! src/main.ts. Drops hang from the surface's edge (notches, bar widgets'
//! dropdowns), each with its springs; every frame they become bumps, the one
//! shape is computed (geometry.rs) and filled, and the content is drawn over it,
//! clipped by it. Logical px throughout; y is the distance from the surface's
//! own edge until it's drawn (a bottom surface is laid out from the bottom: only
//! the shape is mirrored, text is placed, never flipped).

use std::collections::HashMap;
use std::time::Instant;

use serde_json::Value;
use windows::core::Result;
use windows::Win32::Foundation::{COLORREF, HWND, POINT, SIZE};
use windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F;
use windows::Win32::Graphics::Direct2D::{ID2D1Brush, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_LAYER_OPTIONS_NONE, D2D1_LAYER_PARAMETERS};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, SelectObject, AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    BLENDFUNCTION, DIB_RGB_COLORS, HBITMAP, HDC, HGDIOBJ,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows_numerics::Matrix3x2;

use super::geometry::{self, lerp, Bump, Frame, Step};
use super::gfx::{with_alpha, Gfx};
use super::widgets::{self, Act, Button, Ctx, Look, Opts, Panel, Reply, Widget};
use super::Spring;
use crate::config::{Edge, SurfaceMode};
use crate::engine::{Layout, NotchLayout, NotchWidth, Rect};
use crate::win::Monitor;

/// Space between a notch's content and the start of its curves, each side (main.ts).
const NOTCH_PAD: f64 = 10.0;

/// How long a panel's cascade of `n` pieces takes (main.ts cascade).
fn cascade_length(n: usize) -> std::time::Duration {
    let n = n.max(1) as f32;
    let step = (240.0 / n).min(25.0);
    std::time::Duration::from_secs_f32(((n - 1.0) * step + 160.0) / 1000.0)
}

fn clamp01(v: f64) -> f64 {
    v.clamp(0.0, 1.0)
}

/// A widget on this surface: in a notch, or in the open bar's row.
struct Placed {
    name: String,
    w: Box<dyn Widget>,
    opts: Opts,
    /// The notch drop it sits in (bars: it stays there and glides to its place in the open bar).
    notch: Option<usize>,
    /// Its width as last measured, and with the bar open (open-only parts: media keys).
    width: f32,
    open_width: f32,
    /// Bar widgets: x in the open bar's row. Pinned ones: their place there (a spacer), if any.
    bar_x: Option<f32>,
    /// Notch widgets: their x in their notch's slot (from the slot's left).
    slot_x: f32,
    /// Bar widgets that fade in behind pinned ones gliding past them.
    fades: bool,
    /// Where it was last drawn (window px, logical): x, centre y, alpha.
    drawn: Option<(f32, f32, f32)>,
}

/// A panel and its box.
struct PanelBox {
    p: Box<dyn Panel>,
    opts: Opts,
    /// Content size, and the box's (with padding; widened docks' panels).
    content: (f32, f32),
    w: f64,
    h: f64,
    /// Content shifted right (a menu flush in the corner keeps its icons under the widget's).
    shift: f32,
    /// A dock corner panel wider than its content: content along that edge.
    align: Align,
    /// Its content's top-left as last drawn (window px, logical).
    drawn: Option<(f32, f32)>,
    /// When its cascade began (it began opening), until it's shut again.
    cascade: Option<Instant>,
}

#[derive(Clone, Copy, PartialEq)]
enum Align {
    Center,
    Left,
    Right,
}

/// Something that hangs from the surface's edge: a notch, or a bar widget's dropdown.
struct Drop {
    key: String,
    x: f64,
    width: f64,
    height: f64,
    in_hull: bool,
    panel_w: f64,
    panel_h: f64,
    has_panel: bool,
    spring: Spring,
    /// Notches: animated left edge and width (positions from Rust slide).
    xs: Option<Spring>,
    ws: Option<Spring>,
    /// Notches: 1 shown, 0 retracted into the edge (under a window).
    shown: Option<Spring>,
    notch: Option<NotchLayout>,
    /// Notches: the widgets in it, in order.
    slot: Vec<usize>,
    /// Docks: each widget's panel (widget, panel).
    panels: Vec<(usize, usize)>,
    /// The panel it shows (bar dropdowns: always theirs).
    active: Option<usize>,
    /// Bar dropdowns: the widget it hangs from.
    anchor: Option<usize>,
    step: Option<Step>,
}

impl Drop {
    fn new(key: String, spring: Spring) -> Drop {
        Drop {
            key,
            x: 0.0,
            width: 0.0,
            height: 0.0,
            in_hull: false,
            panel_w: 0.0,
            panel_h: 0.0,
            has_panel: false,
            spring,
            xs: None,
            ws: None,
            shown: None,
            notch: None,
            slot: Vec::new(),
            panels: Vec::new(),
            active: None,
            anchor: None,
            step: None,
        }
    }
}

/// The layered window's bitmap.
struct Dib {
    dc: HDC,
    bmp: HBITMAP,
    bits: *mut u8,
    w: i32,
    h: i32,
}

impl Dib {
    fn new(w: i32, h: i32) -> Result<Dib> {
        unsafe {
            let dc = CreateCompatibleDC(None);
            let bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    biHeight: -h,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits = std::ptr::null_mut();
            let bmp = CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0)?;
            SelectObject(dc, bmp.into());
            Ok(Dib { dc, bmp, bits: bits as *mut u8, w, h })
        }
    }
}

impl std::ops::Drop for Dib {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteDC(self.dc);
            let _ = DeleteObject(HGDIOBJ(self.bmp.0));
        }
    }
}

pub struct Surface {
    pub label: String,
    pub hwnd: HWND,
    monitor: Monitor,
    dib: Dib,
    layout: Layout,
    look: Look,
    expanded: bool,
    expand: Spring,
    spread: Spring,
    nudge: Spring,
    nudge_key: Option<String>,
    drops: Vec<Drop>,
    /// Pop-ups out of a notch (OSD, notifications) and the keyboard peek.
    pop: super::popup::Popups,
    widgets: Vec<Placed>,
    panels: Vec<PanelBox>,
    /// Bar mode: where the side groups lie, for the glide of pinned widgets.
    pub animating: bool,
    shown: bool,
    click_through: bool,
    offscreen: bool,
    /// The window's size and position as last drawn (physical).
    size: SIZE,
    pos: POINT,
    tracking: bool,
    /// What the engine was last told, and what's waiting to be told.
    sent_widths: String,
    sent_regions: String,
    pending_widths: Option<HashMap<String, NotchWidth>>,
    pending_regions: Option<Vec<Rect>>,
    /// Needs measuring (content changed size).
    remeasure: bool,
    /// The panel dragging with the mouse captured (a slider).
    captured: Option<usize>,
    /// Acts the surface itself asks for (the launcher's apps).
    pending: Vec<Act>,
}

impl std::ops::Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

impl Surface {
    pub fn new(gfx: &Gfx, label: &str, monitor: Monitor, layout: Layout, store: &HashMap<String, Value>, offscreen: bool) -> Result<Surface> {
        super::register_class();
        let w = monitor.width().max(1);
        let h = ((layout.height * monitor.scale).ceil() as i32).max(1);
        let hwnd = unsafe {
            CreateWindowExW(
                // Click-through until the engine says otherwise (it opens on hover).
                WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT,
                super::SURFACE_CLASS,
                windows::core::w!("Kasumi"),
                WS_POPUP,
                if offscreen { -32000 } else { monitor.left },
                if offscreen { -32000 } else { monitor.top },
                w,
                1,
                None,
                None,
                Some(GetModuleHandleW(None)?.into()),
                None,
            )?
        };
        let a = &layout.animation;
        let mut expand = Spring::new(0.0, a.expand_stiffness, a.expand_damping);
        expand.configure(a.expand_stiffness, a.expand_damping, a);
        let mut spread = Spring::new(0.0, a.spread_stiffness, a.spread_damping);
        spread.configure(a.spread_stiffness, a.spread_damping, a);
        let mut nudge = Spring::new(0.0, 600.0, 30.0);
        nudge.configure(600.0, 30.0, a);
        let mut s = Surface {
            label: label.to_string(),
            hwnd,
            dib: Dib::new(w, h)?,
            monitor,
            look: Look::from_layout(&layout),
            layout: layout.clone(),
            expanded: false,
            expand,
            spread,
            pop: super::popup::Popups::new(a),
            nudge,
            nudge_key: None,
            drops: Vec::new(),
            widgets: Vec::new(),
            panels: Vec::new(),
            animating: false,
            shown: true,
            click_through: true,
            offscreen,
            size: SIZE { cx: w, cy: 1 },
            pos: POINT::default(),
            tracking: false,
            sent_widths: String::new(),
            sent_regions: String::new(),
            pending_widths: None,
            pending_regions: None,
            remeasure: true,
            captured: None,
            pending: Vec::new(),
        };
        s.apply(gfx, layout, store);
        let _ = s.render(gfx);
        if !offscreen {
            unsafe {
                let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            }
        }
        Ok(s)
    }

    fn scale(&self) -> f32 {
        self.monitor.scale as f32
    }

    fn dock(&self) -> bool {
        self.layout.mode == SurfaceMode::Dock
    }

    fn bottom(&self) -> bool {
        self.layout.edge == Edge::Bottom
    }

    /// A widget's options as the page got them (main.ts `opts()`).
    fn opts_for(&self, name: &str) -> Opts {
        let l = &self.layout;
        let owner = if name.starts_with("calendar") { name.replacen("calendar", "clock", 1) } else { name.to_string() };
        let table = l.widgets.get(name).and_then(|v| v.as_table()).cloned().unwrap_or_default();
        let own = l.widgets.get(&owner).and_then(|v| v.as_table());
        let anim = |key: &str, global: bool| own.and_then(|t| t.get(&format!("anim_{key}"))).and_then(|v| v.as_bool()).unwrap_or(global);
        let a = &l.animation;
        Opts {
            table,
            roll: anim("roll", a.rolling),
            magnify: anim("magnify", a.magnify),
            glide: anim("glide", a.marker_glide),
            cascade: anim("cascade", a.cascade),
            nudge: anim("nudge", a.notification_nudge),
            accent: own.and_then(|t| t.get("accent_details")).and_then(|v| v.as_bool()).unwrap_or(l.theme.accent_details),
            font: own.and_then(|t| t.get("font")).and_then(|v| v.as_str()).unwrap_or("").to_string(),
            font2: own.and_then(|t| t.get("font_secondary")).and_then(|v| v.as_str()).unwrap_or("").to_string(),
            device: l.device.clone(),
        }
    }

    fn panel_spring(&self, prev: &HashMap<String, Spring>, key: &str) -> Spring {
        let a = &self.layout.animation;
        let mut s = prev.get(key).cloned().unwrap_or_else(|| Spring::new(0.0, a.panel_stiffness, a.panel_damping));
        s.configure(a.panel_stiffness, a.panel_damping, a);
        s
    }

    /// A new layout (main.ts `apply` + `buildDom`): widgets rebuilt, springs kept by key.
    pub fn apply(&mut self, gfx: &Gfx, layout: Layout, store: &HashMap<String, Value>) {
        let first = self.drops.is_empty() && self.widgets.is_empty();
        let prev: HashMap<String, Spring> = self.drops.iter().map(|d| (d.key.clone(), d.spring.clone())).collect();
        let prev_x: HashMap<String, (Spring, Spring)> = self.drops.iter().filter_map(|d| Some((d.key.clone(), (d.xs.clone()?, d.ws.clone()?)))).collect();
        self.layout = layout;
        self.look = Look::from_layout(&self.layout);
        let a = self.layout.animation.clone();
        self.expand.configure(a.expand_stiffness, a.expand_damping, &a);
        self.spread.configure(a.spread_stiffness, a.spread_damping, &a);
        self.nudge.configure(600.0, 30.0, &a);
        self.pop.configure(&a);
        if first {
            self.expanded = self.layout.expanded;
            self.expand.target = if self.expanded { 1.0 } else { 0.0 };
            self.expand.snap();
        }
        self.spread.target = self.spread_target();
        if first || a.style != "spread" {
            self.spread.snap();
        }

        self.drops.clear();
        self.widgets.clear();
        self.panels.clear();
        let dock = self.dock();
        let notched: Vec<String> = if dock { Vec::new() } else { self.layout.notches.iter().flat_map(|n| n.widgets.clone()).collect() };

        // The open bar's own widgets (bars): each with a dropdown if it has a panel.
        if !dock {
            let bar = self.layout.bar.clone();
            for name in bar.left.iter().chain(&bar.center).chain(&bar.right) {
                if notched.contains(name) || self.widgets.iter().any(|w| &w.name == name) {
                    continue;
                }
                let i = self.add_widget(name, None);
                self.add_dropdown(i, &prev);
            }
        }
        // Notches, with their widgets (one instance each) and panels.
        let notches = self.layout.notches.clone();
        for n in &notches {
            let di = self.drops.len();
            let mut d = Drop::new(format!("notch:{}", n.id), self.panel_spring(&prev, &format!("notch:{}", n.id)));
            d.x = n.x;
            d.width = n.width;
            d.height = n.height;
            d.in_hull = true;
            d.panel_w = n.width;
            let (xs, ws) = prev_x.get(&d.key).cloned().unwrap_or_else(|| (Spring::new(n.x, a.spread_stiffness, a.spread_damping), Spring::new(n.width, a.spread_stiffness, a.spread_damping)));
            let (mut xs, mut ws) = (xs, ws);
            xs.configure(a.spread_stiffness, a.spread_damping, &a);
            ws.configure(a.spread_stiffness, a.spread_damping, &a);
            xs.target = n.x;
            ws.target = n.width;
            d.xs = Some(xs);
            d.ws = Some(ws);
            let on = if self.notch_shown(n) { 1.0 } else { 0.0 };
            let mut shown = Spring::new(on, a.spread_stiffness, a.spread_damping);
            shown.configure(a.spread_stiffness, a.spread_damping, &a);
            d.shown = Some(shown);
            d.notch = Some(n.clone());
            self.drops.push(d);
            for name in &n.widgets {
                let wi = self.add_widget(name, Some(di));
                self.drops[di].slot.push(wi);
                if !dock {
                    self.add_dropdown(wi, &prev);
                } else if let Some(panel) = widgets::panel_for(name) {
                    let opts = self.opts_for(&panel);
                    if let Some(p) = widgets::create_panel(&panel, &opts) {
                        let pi = self.panels.len();
                        self.panels.push(PanelBox { p, opts, content: (0.0, 0.0), w: 0.0, h: 0.0, shift: 0.0, align: Align::Center, drawn: None, cascade: None });
                        self.drops[di].panels.push((wi, pi));
                    }
                }
            }
        }
        let has_widget = self.widgets.iter().any(|w| widgets::widget_type(&w.name) == "launcher");
        if self.layout.launcher && !has_widget {
            let lopts = self.opts_for("launcher");
            if dock {
                let w = self.layout.width;
                let middle = (0..self.drops.len())
                    .filter(|&i| self.drops[i].in_hull && self.drops[i].notch.is_some())
                    .min_by(|&a, &b| {
                        let c = |i: usize| (self.drops[i].x + self.drops[i].width / 2.0 - w / 2.0).abs();
                        c(a).total_cmp(&c(b))
                    });
                if let Some(di) = middle {
                    self.widgets.push(Placed { name: "launcher".into(), w: Box::new(widgets::launcher::LauncherWidget::new(&lopts, true)), opts: lopts.clone(), notch: Some(di), width: 0.0, open_width: 0.0, bar_x: None, slot_x: 0.0, fades: false, drawn: None });
                    let wi = self.widgets.len() - 1;
                    self.drops[di].slot.push(wi);
                    let pi = self.panels.len();
                    self.panels.push(PanelBox { p: Box::new(widgets::launcher::LauncherPanel::new(&lopts, false)), opts: lopts, content: (0.0, 0.0), w: 0.0, h: 0.0, shift: 0.0, align: Align::Center, drawn: None, cascade: None });
                    self.drops[di].panels.push((wi, pi));
                }
            } else {
                let pi = self.panels.len();
                self.panels.push(PanelBox { p: Box::new(widgets::launcher::LauncherPanel::new(&lopts, true)), opts: lopts, content: (0.0, 0.0), w: 0.0, h: 0.0, shift: 0.0, align: Align::Center, drawn: None, cascade: None });
                let mut d = Drop::new("bar:launcher".into(), self.panel_spring(&prev, "bar:launcher"));
                d.x = self.layout.width / 2.0;
                d.has_panel = true;
                d.active = Some(pi);
                self.drops.push(d);
            }
        }
        // Pop-up notches for spots without a notch ([osd] spot): zero-size until a pop-up
        // grows out of one; corners reach past the screen edge like a notch hugging one.
        let r = self.layout.geometry.notch_corner_radius;
        let w = self.layout.width;
        for (side, x) in [("left", -r), ("center", w / 2.0), ("right", w + r)] {
            let mut d = Drop::new(format!("spot:{side}"), Spring::new(0.0, 1.0, 1.0));
            d.x = x;
            self.drops.push(d);
        }
        // The keyboard peek: a zero-size bump until it pops (x holds its centre).
        self.drops.push(Drop::new("peek".into(), Spring::new(0.0, 1.0, 1.0)));
        // Data the widgets read, as it stands.
        for w in &mut self.widgets {
            for key in w.w.keys() {
                if let Some(v) = store.get(*key) {
                    w.w.data(key, v);
                }
            }
        }
        for p in &mut self.panels {
            for key in p.p.keys() {
                if let Some(v) = store.get(*key) {
                    p.p.data(key, v);
                }
            }
        }
        self.sent_widths.clear();
        self.remeasure = true;
        self.measure(gfx);
        self.kick();
    }

    fn add_widget(&mut self, name: &str, notch: Option<usize>) -> usize {
        let opts = self.opts_for(name);
        let w = widgets::create(name, opts.clone());
        self.widgets.push(Placed { name: name.to_string(), w, opts, notch, width: 0.0, open_width: 0.0, bar_x: None, slot_x: 0.0, fades: false, drawn: None });
        self.widgets.len() - 1
    }

    /// A widget with a panel gets a dropdown under it in the open bar.
    fn add_dropdown(&mut self, wi: usize, prev: &HashMap<String, Spring>) {
        let name = self.widgets[wi].name.clone();
        let Some(panel) = widgets::panel_for(&name) else { return };
        let opts = self.opts_for(&panel);
        let Some(p) = widgets::create_panel(&panel, &opts) else { return };
        let pi = self.panels.len();
        self.panels.push(PanelBox { p, opts, content: (0.0, 0.0), w: 0.0, h: 0.0, shift: 0.0, align: Align::Center, drawn: None, cascade: None });
        let key = format!("bar:{name}");
        let mut d = Drop::new(key.clone(), self.panel_spring(prev, &key));
        d.has_panel = true;
        d.active = Some(pi);
        d.anchor = Some(wi);
        self.drops.push(d);
    }

    /// "drop" keeps the bar full width so it slides out of the edge; "spread" grows it sideways.
    fn spread_target(&self) -> f64 {
        if self.layout.animation.style == "spread" && !self.expanded {
            0.0
        } else {
            1.0
        }
    }

    /// Whether a notch shows now: not retracted, and (docks) pinned or the dock open.
    fn notch_shown(&self, n: &NotchLayout) -> bool {
        !n.hidden && !(n.closed_hidden && !self.expanded)
    }

    pub fn set_expanded(&mut self, gfx: &Gfx, expanded: bool) {
        self.expanded = expanded;
        for i in 0..self.drops.len() {
            if let Some(n) = self.drops[i].notch.clone() {
                let on = if self.notch_shown(&n) { 1.0 } else { 0.0 };
                if let Some(s) = self.drops[i].shown.as_mut() {
                    s.target = on;
                }
            }
        }
        if expanded {
            self.nudge.target = 0.0;
        }
        self.expand.target = if expanded { 1.0 } else { 0.0 };
        self.spread.target = self.spread_target();
        if !expanded {
            for d in &mut self.drops {
                d.spring.target = 0.0;
            }
        }
        self.layout_bar(gfx);
        self.kick();
        self.update_regions();
    }

    /// Rust moved notches (a docking notch left its corner): slide them over.
    pub fn update_positions(&mut self, gfx: &Gfx, notches: Vec<NotchLayout>) {
        for n in notches {
            if let Some(m) = self.layout.notches.iter_mut().find(|m| m.id == n.id) {
                *m = n.clone();
            }
            let shown = self.notch_shown(&n);
            if let Some(d) = self.drops.iter_mut().find(|d| d.key == format!("notch:{}", n.id)) {
                d.x = n.x;
                d.width = n.width;
                if let Some(xs) = d.xs.as_mut() {
                    xs.target = n.x;
                }
                if let Some(ws) = d.ws.as_mut() {
                    ws.target = n.width;
                }
                if let Some(s) = d.shown.as_mut() {
                    s.target = if shown { 1.0 } else { 0.0 };
                }
                d.notch = Some(n);
            }
        }
        self.layout_slots();
        self.layout_bar(gfx);
        self.measure_panels(gfx);
        self.kick();
        self.update_regions();
    }

    /// Hover nudge: the notch the cursor is coming near (None: none).
    pub fn set_nudge(&mut self, notch: Option<String>) {
        if let Some(n) = &notch {
            let key = format!("notch:{n}");
            if self.nudge_key.as_deref() != Some(&key) {
                self.nudge.value = 0.0;
            }
            self.nudge_key = Some(key);
        }
        self.nudge.target = if notch.is_some() { 1.0 } else { 0.0 };
        self.kick();
    }

    /// Pop-ups and the launcher (later phases).
    pub fn event(&mut self, gfx: &Gfx, event: &str, payload: &Value) {
        match event {
            "notch://osd" => self.pop.osd(gfx, &self.look, &self.layout, payload),
            "notch://notification" => self.pop.notification(gfx, &self.look, &self.layout, payload),
            "notch://keyboard" => self.keyboard(gfx, payload),
            "notch://launcher" => {
                let open = payload.get("open").and_then(|o| o.as_bool()).unwrap_or(false);
                self.set_launcher(gfx, open);
            }
            _ => return,
        }
        self.kick();
    }

    /// Rust opened (focused this surface) or closed the launcher.
    fn set_launcher(&mut self, gfx: &Gfx, open: bool) {
        widgets::launcher::set_open(open);
        if !open {
            self.close_all();
        } else {
            self.pending.push(widgets::launcher::load_apps());
            // Its panel: a launcher widget's, the dock's middle notch's, or the bar's own.
            let wi = self.widgets.iter().position(|w| widgets::widget_type(&w.name) == "launcher");
            match wi {
                Some(wi) => self.panel_of(gfx, wi, Some(true)),
                None => {
                    if let Some(di) = self.drops.iter().position(|d| d.key == "bar:launcher") {
                        for (i, d) in self.drops.iter_mut().enumerate() {
                            d.spring.target = if i == di { 1.0 } else { 0.0 };
                        }
                    }
                }
            }
        }
        // The field grows the notch (its other widgets step aside): measure again.
        self.remeasure = true;
        self.measure(gfx);
        self.kick();
        self.update_regions();
    }

    pub fn launcher_open(&self) -> bool {
        widgets::launcher::searching() && self.widgets.iter().any(|w| widgets::widget_type(&w.name) == "launcher") || (widgets::launcher::searching() && self.drops.iter().any(|d| d.key == "bar:launcher" && d.spring.target > 0.5))
    }

    /// A key while the launcher's field has the focus.
    pub fn key(&mut self, gfx: &Gfx, vk: u16, ctrl: bool) -> Option<Act> {
        if ctrl && vk == 0x56 {
            if let Some(t) = widgets::launcher::clipboard_text() {
                widgets::launcher::paste(&t);
                self.changed(gfx);
            }
            return None;
        }
        match widgets::launcher::key(vk, ctrl, self.bottom()) {
            widgets::launcher::Out::None => None,
            widgets::launcher::Out::Redraw => {
                self.changed(gfx);
                None
            }
            widgets::launcher::Out::Launch(id) => self.answer(gfx, Reply::Do(widgets::launcher::launch(id))),
            widgets::launcher::Out::Close => self.answer(gfx, Reply::Do(Act::Launcher { open: false, refocus: true })),
        }
    }

    /// Typed text (WM_CHAR).
    pub fn char(&mut self, gfx: &Gfx, c: u32) -> Option<Act> {
        if char::from_u32(c).is_some_and(widgets::launcher::type_char) {
            self.changed(gfx);
        }
        None
    }

    /// The keyboard layout changed: the peek at the language widget, or the pill.
    fn keyboard(&mut self, gfx: &Gfx, e: &Value) {
        let format = self.layout.widgets.get("language").and_then(|w| w.get("format")).and_then(|v| v.as_str()).map(String::from);
        let code = super::widgets::language_code(e, format.as_deref());
        let word = serde_json::json!({ "kind": "keyboard", "level": 0, "muted": false, "text": code, "notch": e.get("notch") });
        // Out of a notch like the other pop-ups ([osd] keyboard_spot), not the peek.
        if e.get("notch").is_some_and(|n| !n.is_null()) {
            self.pop.osd(gfx, &self.look, &self.layout, &word);
            return;
        }
        // The language widget shows the new layout itself whenever it can be seen.
        let lang = self.widgets.iter().find(|w| w.name == "language");
        let in_notch = self.layout.notches.iter().find(|n| n.widgets.iter().any(|w| w == "language"));
        let visible = (self.expanded && lang.is_some())
            || in_notch.is_some_and(|n| !n.hidden && !n.hidden_widgets.iter().any(|w| w == "language") && (!self.dock() || (!n.shrunk && self.layout.rest != crate::config::SizeName::Tiny)));
        if visible {
            return;
        }
        // Docks (and an open bar): the pill rather than a second bump.
        let at = lang.and_then(|w| w.drawn.map(|(x, _, _)| x as f64 + w.width as f64 / 2.0));
        match at {
            Some(x) if !self.expanded && !self.dock() => self.pop.peek(code, x, self.layout.osd.duration_ms),
            _ => self.pop.osd(gfx, &self.look, &self.layout, &serde_json::json!({ "kind": "keyboard", "level": 0, "muted": false, "text": code })),
        }
    }

    pub fn data(&mut self, gfx: &Gfx, key: &str, v: &Value) {
        if key == "appIcons" && self.pop.data(v) && self.pop.show.is_some() {
            self.redraw(gfx);
        }
        let mut changed = false;
        for w in &mut self.widgets {
            if w.w.keys().contains(&key) {
                changed |= w.w.data(key, v);
            }
        }
        for p in &mut self.panels {
            if p.p.keys().contains(&key) {
                changed |= p.p.data(key, v);
            }
        }
        if changed {
            self.changed(gfx);
        }
    }

    /// Content changed: measure again, and draw (frames while something rolls).
    fn changed(&mut self, gfx: &Gfx) {
        self.remeasure = true;
        self.measure(gfx);
        let now = Instant::now();
        if self.widgets.iter().any(|w| w.w.animating(now)) {
            self.kick();
        } else {
            self.redraw(gfx);
        }
    }

    /// Widgets' timers.
    /// What panels asked for in their ticks.
    pub fn take_acts(&mut self) -> Vec<Act> {
        let mut acts: Vec<Act> = self.panels.iter_mut().filter_map(|p| p.p.act()).collect();
        acts.extend(self.pop.pending.take());
        acts.append(&mut self.pending);
        acts
    }

    pub fn tick(&mut self, gfx: &Gfx, now: Instant) {
        if self.pop.tick(now) {
            self.kick();
        }
        let mut changed = false;
        let mut redraw = false;
        for w in &mut self.widgets {
            if w.w.wake().is_some_and(|t| t <= now) {
                let looks = w.w.tick(now);
                if w.w.tick_resizes() {
                    changed |= looks;
                } else {
                    redraw |= looks;
                }
            }
        }

        for p in &mut self.panels {
            if p.p.wake().is_some_and(|t| t <= now) {
                let looks = p.p.tick(now);
                if p.p.tick_resizes() {
                    changed |= looks;
                } else {
                    redraw |= looks;
                }
            }
        }
        if changed {
            self.changed(gfx);
        } else if redraw {
            self.redraw(gfx);
        }
    }

    pub fn wake(&self) -> Option<Instant> {
        self.widgets.iter().filter_map(|w| w.w.wake()).chain(self.panels.iter().filter_map(|p| p.p.wake())).chain(self.pop.wake()).min()
    }

    pub fn set_shown(&mut self, shown: bool) {
        self.shown = shown;
        if !self.offscreen {
            unsafe {
                let _ = ShowWindow(self.hwnd, if shown { SW_SHOWNOACTIVATE } else { SW_HIDE });
            }
        }
    }

    pub fn set_click_through(&mut self, ignore: bool) {
        if ignore == self.click_through {
            return;
        }
        self.click_through = ignore;
        unsafe {
            let ex = GetWindowLongW(self.hwnd, GWL_EXSTYLE) as u32;
            let ex = if ignore { ex | WS_EX_TRANSPARENT.0 } else { ex & !WS_EX_TRANSPARENT.0 };
            SetWindowLongW(self.hwnd, GWL_EXSTYLE, ex as i32);
        }
    }

    pub fn place(&mut self, monitor: Monitor) {
        if monitor.width() != self.monitor.width() {
            if let Ok(d) = Dib::new(monitor.width().max(1), self.dib.h) {
                self.dib = d;
            }
        }
        self.monitor = monitor;
        self.look = Look::from_layout(&self.layout);
        self.kick();
    }

    fn kick(&mut self) {
        self.animating = true;
    }

    /// Redraws now unless a frame is coming anyway.
    fn redraw(&mut self, gfx: &Gfx) {
        if !self.animating {
            let _ = self.render(gfx);
        }
    }

    // ------------------------------------------------------------------
    // Measuring: notch widths for the engine, widgets' places in the open bar,
    // panels' sizes.

    /// Extra room on a corner-hugging notch's outer side, up to the bar's edge padding (bars only).
    fn edge_extra(&self, n: &NotchLayout) -> f64 {
        if n.edge.is_some() && self.layout.mode == SurfaceMode::Bar {
            (self.layout.bar.padding - NOTCH_PAD).max(0.0)
        } else {
            0.0
        }
    }

    fn hides(&self, name: &str) -> bool {
        self.layout.widgets.get(name).and_then(|v| v.get("hide_when_maximized")).and_then(|v| v.as_bool()) == Some(true)
    }

    /// A widget stepping aside for the launcher's field in its notch (while it searches).
    fn aside_in(&self, d: &Drop, wi: usize) -> bool {
        widgets::launcher::searching() && widgets::widget_type(&self.widgets[wi].name) != "launcher" && d.slot.iter().any(|&k| widgets::widget_type(&self.widgets[k].name) == "launcher")
    }

    fn aside(&self, di: usize, wi: usize) -> bool {
        self.aside_in(&self.drops[di], wi)
    }

    fn covered(&self, d: &Drop, wi: usize) -> bool {
        d.notch.as_ref().is_some_and(|n| n.hidden_widgets.contains(&self.widgets[wi].name))
    }

    /// Widths of everything, notch widths to report if they changed, places in the bar.
    fn measure(&mut self, gfx: &Gfx) {
        if !self.remeasure {
            return;
        }
        self.remeasure = false;
        let l = &self.layout;
        let dock = self.dock();
        let (rest_h, open_h) = (l.rest_height as f32, l.open_height as f32);
        // Widths at the resting size (docks: widgets may size with the row).
        let mut widths = Vec::with_capacity(self.widgets.len());
        let mut open_widths = Vec::with_capacity(self.widgets.len());
        for i in 0..self.widgets.len() {
            let opts = self.widgets[i].opts.clone();
            let row = if dock { rest_h } else { l.geometry.notch_height as f32 };
            let w0 = {
                let cx = Ctx { gfx, look: &self.look, opts: &opts, h: row, open: 0.0 };
                self.widgets[i].w.width(&cx)
            };
            let w1 = if dock {
                let cx = Ctx { gfx, look: &self.look, opts: &opts, h: open_h, open: 1.0 };
                self.widgets[i].w.width(&cx)
            } else {
                w0
            };
            let w1 = if dock {
                w1
            } else {
                let cx = Ctx { gfx, look: &self.look, opts: &opts, h: l.geometry.expanded_height as f32, open: 1.0 };
                self.widgets[i].w.width(&cx)
            };
            widths.push(w0);
            open_widths.push(w1);
        }
        for (i, w) in widths.iter().enumerate() {
            self.widgets[i].width = *w;
            self.widgets[i].open_width = open_widths[i];
        }
        // Each notch's content width (plus its curves and padding), at rest and open.
        let g = &self.layout.geometry;
        let gap = self.layout.bar.gap as f32;
        let base = |h: f64| 2.0 * g.notch_corner_radius * (if dock { h / g.notch_height } else { 1.0 }) + 2.0 * NOTCH_PAD;
        let mut report: HashMap<String, NotchWidth> = HashMap::new();
        for d in &self.drops {
            let Some(n) = &d.notch else { continue };
            let kids: Vec<usize> = d.slot.iter().copied().filter(|&wi| widths[wi] > 0.0 && !self.aside_in(d, wi)).collect();
            let span = |ws: &[f32], list: &[usize], pad: f64| -> f64 {
                if list.is_empty() {
                    return 0.0;
                }
                let sum: f32 = list.iter().map(|&wi| ws[wi]).sum::<f32>() + gap * (list.len() as f32 - 1.0);
                (sum as f64 + pad + self.edge_extra(n)).ceil()
            };
            let full = span(&widths, &kids, base(l.rest_height));
            let compact_list: Vec<usize> = kids.iter().copied().filter(|&wi| !self.hides(&self.widgets[wi].name)).collect();
            let compact = span(&widths, &compact_list, base(l.rest_height));
            let open = if dock { span(&open_widths, &d.slot.iter().copied().filter(|&wi| open_widths[wi] > 0.0).collect::<Vec<_>>(), base(l.open_height)) } else { full };
            if full > 0.0 || open > 0.0 {
                report.insert(n.id.clone(), NotchWidth { full, compact, open });
            }
        }
        let mut keys: Vec<_> = report.iter().map(|(k, w)| format!("{k}:{}:{}:{}", w.full, w.compact, w.open)).collect();
        keys.sort();
        let sent = keys.join(";");
        if !report.is_empty() && sent != self.sent_widths {
            self.sent_widths = sent;
            self.pending_widths = Some(report);
        }
        self.layout_slots();
        self.layout_bar(gfx);
        self.measure_panels(gfx);
    }

    /// Notch widgets' places in their slot (closed): centred, past the edge padding.
    fn layout_slots(&mut self) {
        let gap = self.layout.bar.gap as f32;
        for di in 0..self.drops.len() {
            let Some(n) = self.drops[di].notch.clone() else { continue };
            let kids: Vec<usize> = self.drops[di].slot.iter().copied().filter(|&wi| self.widgets[wi].width > 0.0 && !self.covered(&self.drops[di], wi) && !self.aside(di, wi)).collect();
            let cw: f32 = kids.iter().map(|&wi| self.widgets[wi].width).sum::<f32>() + gap * (kids.len().max(1) as f32 - 1.0);
            let (pl, pr) = self.slot_pads(&n, 1.0);
            let avail = n.width as f32 - pl - pr;
            let mut x = pl + (avail - cw) / 2.0;
            for wi in kids {
                self.widgets[wi].slot_x = x;
                x += self.widgets[wi].width + gap;
            }
        }
    }

    /// A slot's padding on each side: a corner notch's content keeps to the corner,
    /// past its part beyond the screen edge, its round-off and padding.
    fn slot_pads(&self, n: &NotchLayout, k: f64) -> (f32, f32) {
        let inset = (self.edge_extra(n) + self.layout.geometry.notch_corner_radius * k + NOTCH_PAD) as f32;
        match n.edge.as_deref() {
            Some("left") => (inset, 0.0),
            Some("right") => (0.0, inset),
            _ => (0.0, 0.0),
        }
    }

    /// Bars: the open bar's row (main.ts layoutBar): left and right groups from the
    /// edges (pinned widgets keep their place, a spacer they glide to), centre ones
    /// in the middle or around a middle notch.
    fn layout_bar(&mut self, _gfx: &Gfx) {
        if self.dock() {
            return;
        }
        let l = &self.layout;
        let gap = l.bar.gap as f32;
        let pad = l.bar.padding as f32;
        let w = l.width as f32;
        let inset = (l.geometry.notch_corner_radius + NOTCH_PAD) as f32;
        let find = |name: &str| self.widgets.iter().position(|p| p.name == name);
        let notch_of = |name: &str| self.layout.notches.iter().find(|n| n.widgets.iter().any(|x| x == name));
        // A pinned widget shows if its notch does and it isn't hidden under a window.
        let pinned_shown = |wi: usize| {
            let p = &self.widgets[wi];
            p.open_width > 0.0 && notch_of(&p.name).is_some_and(|n| !n.hidden && !n.hidden_widgets.contains(&p.name))
        };
        let mut places: Vec<(usize, f32)> = Vec::new();
        let mut fades: Vec<usize> = Vec::new();
        let group = |names: &[String], include_pinned: bool| -> Vec<usize> {
            names
                .iter()
                .filter_map(|n| find(n))
                .filter(|&wi| {
                    let p = &self.widgets[wi];
                    if p.notch.is_some() {
                        include_pinned && pinned_shown(wi)
                    } else {
                        p.open_width > 0.0
                    }
                })
                .collect()
        };
        let span = |list: &[usize]| -> f32 { list.iter().map(|&wi| self.widgets[wi].open_width).sum::<f32>() + gap * (list.len().max(1) as f32 - 1.0) };
        let left = group(&l.bar.left, true);
        let right = group(&l.bar.right, true);
        let mut x = pad;
        for &wi in &left {
            places.push((wi, x));
            x += self.widgets[wi].open_width + gap;
        }
        let mut x = w - pad - span(&right);
        for &wi in &right {
            places.push((wi, x));
            x += self.widgets[wi].open_width + gap;
        }
        for side in [&left, &right] {
            if side.iter().any(|&wi| self.widgets[wi].notch.is_some()) {
                fades.extend(side.iter().copied().filter(|&wi| self.widgets[wi].notch.is_none()));
            }
        }
        // Centre: around a middle notch (widgets listed before its own sit left of it).
        let middle = l.notches.iter().filter(|n| !n.hidden && n.widgets.iter().any(|x| l.bar.center.contains(x))).last();
        let first_pinned = l.bar.center.iter().position(|n| l.notches.iter().any(|m| m.widgets.contains(n)));
        match (middle, first_pinned) {
            (Some(m), Some(first)) => {
                let before = group(&l.bar.center[..first], false);
                let after = group(&l.bar.center[first..], false);
                let x0 = (m.x as f32) + inset;
                let x1 = (m.x + m.width) as f32 - inset;
                let mut x = x0 - gap - span(&before);
                for &wi in &before {
                    places.push((wi, x));
                    x += self.widgets[wi].open_width + gap;
                }
                let mut x = x1 + gap;
                for &wi in &after {
                    places.push((wi, x));
                    x += self.widgets[wi].open_width + gap;
                }
            }
            _ => {
                let center = group(&l.bar.center, false);
                let mut x = w / 2.0 - span(&center) / 2.0;
                for &wi in &center {
                    places.push((wi, x));
                    x += self.widgets[wi].open_width + gap;
                }
            }
        }
        for p in &mut self.widgets {
            p.bar_x = None;
            p.fades = false;
        }
        for (wi, x) in places {
            self.widgets[wi].bar_x = Some(x.round());
        }
        for wi in fades {
            self.widgets[wi].fades = true;
        }
    }

    /// Panels' sizes, and where bar dropdowns hang (main.ts measureBarDrops).
    fn measure_panels(&mut self, gfx: &Gfx) {
        let dock = self.dock();
        let l = self.layout.clone();
        for pi in 0..self.panels.len() {
            let opts = self.panels[pi].opts.clone();
            let size = {
                let cx = Ctx { gfx, look: &self.look, opts: &opts, h: l.geometry.expanded_height as f32, open: 1.0 };
                self.panels[pi].p.size(&cx)
            };
            let (_, _, px) = self.panels[pi].p.pad();
            let (near, far, _) = self.panels[pi].p.pad();
            let pb = &mut self.panels[pi];
            pb.content = size;
            pb.shift = 0.0;
            pb.align = Align::Center;
            pb.w = (size.0 + 2.0 * px) as f64;
            pb.h = (size.1 + near + far) as f64;
        }
        if let Some(di) = self.drops.iter().position(|d| d.key == "bar:launcher") {
            if let Some(pi) = self.drops[di].active {
                let (w, h) = (self.panels[pi].w, self.panels[pi].h);
                let d = &mut self.drops[di];
                d.x = l.width / 2.0;
                d.panel_w = w;
                d.panel_h = h;
            }
        }
        for di in 0..self.drops.len() {
            if let Some(wi) = self.drops[di].anchor {
                let Some(pi) = self.drops[di].active else { continue };
                // Under the widget where it ends up in the open bar.
                let Some(ax) = self.open_x(wi) else { continue };
                let ww = self.widgets[wi].open_width;
                let pb = &mut self.panels[pi];
                let (_, _, px) = pb.p.pad();
                let x = match (self.widgets[wi].w.icon_x(), pb.p.icon_x()) {
                    // Panels that opt in (the menu) line their icon column up under the widget's icon.
                    (Some(wicon), Some(picon)) => {
                        let mut left = ax + wicon - (px + picon);
                        // Just short of the screen's edge: flush with it instead, its content shifted.
                        if left > 0.0 && left < 24.0 {
                            pb.shift = left;
                            pb.w += left as f64;
                            left = 0.0;
                        }
                        left as f64 + pb.w / 2.0
                    }
                    _ => (ax + ww / 2.0) as f64,
                };
                let d = &mut self.drops[di];
                d.x = x;
                d.panel_w = pb.w;
                d.panel_h = pb.h;
                continue;
            }
            if !dock {
                continue;
            }
            // Dock notch panels: at least as wide as the open notch (corner ones may rise beside it).
            let Some(n) = self.drops[di].notch.clone() else { continue };
            let Some(pi) = self.drops[di].active.or_else(|| self.drops[di].panels.first().map(|p| p.1)) else { continue };
            let side = n.edge.clone();
            let mut row = n.open_width;
            if side.as_deref() == Some("left") {
                row = n.open_x + n.open_width;
            }
            if side.as_deref() == Some("right") {
                row = l.width - n.open_x;
            }
            let scale = n.open_height / l.geometry.notch_height;
            let pb = &mut self.panels[pi];
            let room = pb.w + l.geometry.panel_concave_radius + l.geometry.notch_corner_radius * scale;
            let step = match side.as_deref() {
                Some("left") if room <= row => Some(Step::Left),
                Some("right") if room <= row => Some(Step::Right),
                _ => None,
            };
            if step.is_none() && pb.w < row {
                pb.align = match side.as_deref() {
                    Some("left") => Align::Left,
                    Some("right") => Align::Right,
                    _ => Align::Center,
                };
                pb.w = row;
            }
            let d = &mut self.drops[di];
            d.step = step;
            d.panel_w = pb.w;
            d.panel_h = pb.h;
            d.has_panel = true;
        }
    }

    /// Where a widget sits once the bar is open: its place in the bar's row, else its notch.
    fn open_x(&self, wi: usize) -> Option<f32> {
        let p = &self.widgets[wi];
        if let Some(x) = p.bar_x {
            return Some(x);
        }
        let d = &self.drops[p.notch?];
        let x = d.xs.as_ref().map_or(d.x, |s| s.target) as f32;
        Some(x + p.slot_x)
    }

    // ------------------------------------------------------------------
    // Frames.

    /// Drops in path order (left to right by collapsed centre).
    fn ordered(&self) -> Vec<usize> {
        let mut idx: Vec<usize> = (0..self.drops.len()).collect();
        idx.sort_by(|&a, &b| (self.drops[a].x + self.drops[a].width / 2.0).total_cmp(&(self.drops[b].x + self.drops[b].width / 2.0)));
        idx
    }

    /// How far a notch's widgets glide from it to their places in the open bar (on average).
    fn glide_shift(&self, di: usize, x: f64) -> f64 {
        let d = &self.drops[di];
        let mine: Vec<f64> = d.slot.iter().filter_map(|&wi| self.widgets[wi].bar_x.map(|to| to as f64 - (x + self.widgets[wi].slot_x as f64))).collect();
        if mine.is_empty() {
            0.0
        } else {
            mine.iter().sum::<f64>() / mine.len() as f64
        }
    }

    /// The drop a pop-up grows out of: the notch Rust picked ([osd] spot), else the centre one.
    fn osd_drop(&self) -> Option<usize> {
        let by_key = |k: &str| self.drops.iter().position(|d| d.key == k);
        self.pop
            .notch
            .as_deref()
            .and_then(|n| by_key(&format!("notch:{n}")).or_else(|| by_key(n)))
            .or_else(|| by_key("notch:center"))
            .or_else(|| self.drops.iter().position(|d| d.in_hull))
    }

    /// Whether the pop-up drops below the open bar (it would hide a grown notch) instead.
    fn osd_below(&self, use_target: bool) -> bool {
        !self.dock() && (if use_target { self.expand.target } else { self.expand.value }) > 0.5
    }

    /// The corner a drop hugs (-1 left, 1 right, 0 none).
    fn side_of(&self, di: usize) -> i32 {
        let d = &self.drops[di];
        match (d.key.as_str(), d.notch.as_ref().and_then(|n| n.edge.as_deref())) {
            ("spot:left", _) | (_, Some("left")) => -1,
            ("spot:right", _) | (_, Some("right")) => 1,
            _ => 0,
        }
    }

    /// The pop-up pill's size (a word's or a notification's, else [osd] width x height).
    fn osd_size(&self) -> (f64, f64) {
        self.pop.size.unwrap_or((self.layout.osd.width, self.layout.osd.height))
    }

    fn bumps(&self, order: &[usize], use_target: bool) -> Vec<Bump> {
        let l = &self.layout;
        let dock = self.dock();
        let val = |s: &Spring| if use_target { s.target } else { s.value };
        let od = self.osd_drop();
        let o = val(&self.pop.osd).max(0.0);
        let below = self.osd_below(use_target);
        let (ow, oh) = self.osd_size();
        order
            .iter()
            .map(|&di| {
                let d = &self.drops[di];
                if d.key == "peek" {
                    let k = val(&self.pop.peek).max(0.0);
                    let width = l.osd.keyboard_width * k;
                    return Bump { x: self.pop.peek_x - width / 2.0, width, height: l.geometry.notch_height * k, panel_width: width, ..Default::default() };
                }
                let mut x = d.xs.as_ref().map_or(d.x, val);
                if !dock {
                    x += self.glide_shift(di, x) * clamp01((val(&self.expand) - 0.85) / 0.15);
                }
                let mut width = d.ws.as_ref().map_or(d.width, val);
                let mut height = d.height;
                let nudged = if self.nudge_key.as_deref() == Some(&d.key) { val(&self.nudge).max(0.0) } else { 0.0 };
                let mut scale = 1.0;
                let mut panel_top = None;
                if let (true, Some(n)) = (dock, &d.notch) {
                    // Grow from the rest size to the open size in place.
                    let e = val(&self.expand).max(0.15 * nudged).clamp(0.0, 1.0);
                    x = lerp(x, n.open_x, e);
                    width = lerp(width, n.open_width, e);
                    height = lerp(n.height, n.open_height, e);
                    scale = height / l.geometry.notch_height;
                    panel_top = Some(height);
                } else if nudged > 0.0 {
                    x -= 5.0 * nudged;
                    width += 10.0 * nudged;
                    height += 4.0 * nudged;
                }
                if let Some(s) = &d.shown {
                    // Retracting (hidden under a window): shrink into the edge around the centre.
                    let s = clamp01(val(s));
                    x += width * (1.0 - s) / 2.0;
                    width *= s;
                    height *= s;
                }
                let is_osd = od == Some(di) && o > 0.0;
                if is_osd && !below {
                    // Grow from the notch's side: a corner one keeps its outer edge (the pill
                    // stays on screen), the others their centre.
                    let from = x;
                    width = d.width + (ow - d.width) * o;
                    height = d.height + (oh - d.height) * o;
                    x = match self.side_of(di) {
                        -1 => from,
                        1 => from + d.width - width,
                        _ => (from + d.width / 2.0 - width / 2.0).clamp(0.0, (l.width - width).max(0.0)),
                    };
                }
                let osd_panel = is_osd && below;
                Bump {
                    x,
                    width,
                    height,
                    panel_width: if osd_panel { ow } else { d.panel_w },
                    panel_height: if osd_panel { oh } else { d.panel_h },
                    p: if osd_panel { o } else if d.has_panel && d.active.is_some() { val(&d.spring) } else { 0.0 },
                    in_hull: d.in_hull,
                    rounded: None,
                    scale,
                    panel_top,
                    step: if dock { d.step } else { None },
                }
            })
            .collect()
    }

    fn frame_of(&self, order: &[usize], use_target: bool) -> (Vec<Bump>, Frame) {
        let bumps = self.bumps(order, use_target);
        let dock = self.dock();
        let l = &self.layout;
        let val = |s: &Spring| if use_target { s.target } else { s.value };
        let frame = geometry::compute(
            &l.geometry,
            l.width,
            l.corner_radius,
            l.combine_notches,
            if dock { 0.0 } else { val(&self.expand) },
            if dock { 0.0 } else { val(&self.spread) },
            &bumps,
        );
        (bumps, frame)
    }

    /// Panels' cascades: one starts as its panel begins to open, and resets once it's shut.
    fn update_cascades(&mut self) {
        for d in &self.drops {
            let Some(pi) = d.active else { continue };
            let pb = &mut self.panels[pi];
            if d.spring.value <= 0.01 && d.spring.target < 0.5 {
                pb.cascade = None;
            } else if pb.cascade.is_none() && pb.opts.cascade {
                pb.cascade = Some(Instant::now());
            }
        }
    }

    /// One animation step (once per display refresh while something moves).
    pub fn frame(&mut self, gfx: &Gfx, dt: f64) {
        let now = Instant::now();
        let panels_moving = self.panels.iter().any(|p| p.p.animating(now));
        if panels_moving {
            // A list sliding open inside a panel: the panel's shape follows every frame.
            self.measure_panels(gfx);
        }
        let bouncing = self.pop.bouncing(now);
        let mut springs: Vec<&mut Spring> = vec![&mut self.expand, &mut self.spread, &mut self.nudge, &mut self.pop.osd, &mut self.pop.peek];
        for d in &mut self.drops {
            springs.push(&mut d.spring);
            if let Some(s) = d.xs.as_mut() {
                springs.push(s);
            }
            if let Some(s) = d.ws.as_mut() {
                springs.push(s);
            }
            if let Some(s) = d.shown.as_mut() {
                springs.push(s);
            }
        }
        for s in springs.iter_mut() {
            s.step(dt);
        }
        // Cascades run on time (a spring is nearly open within ~100 ms): frames until they're done.
        let cascading = self.panels.iter().any(|p| p.cascade.is_some_and(|t0| now < t0 + cascade_length(p.p.pieces().len())));
        let busy = self.widgets.iter().any(|w| w.w.animating(now)) || panels_moving || cascading || bouncing;
        if springs.iter().all(|s| s.settled()) && !busy {
            for s in springs.iter_mut() {
                s.snap();
            }
            self.animating = false;
            // Panels that have shut forget their hover / armed state.
            for d in &self.drops {
                if d.spring.value <= 0.0 {
                    if let Some(pi) = d.active {
                        self.panels[pi].p.closed();
                    }
                }
            }
        }
        let _ = self.render(gfx);
    }

    /// Draws the surface into its window, only as tall as what shows.
    pub fn render(&mut self, gfx: &Gfx) -> Result<()> {
        self.update_cascades();
        let order = self.ordered();
        let (bumps, frame) = self.frame_of(&order, false);
        // Docks: widgets size with the row as it grows (the pages' --h / --t).
        let ex = clamp01(self.expand.value) as f32;
        let row_h_dock = lerp(self.layout.rest_height, self.layout.open_height, ex as f64) as f32;
        let dock_widths: Vec<f32> = if self.dock() {
            let look = &self.look;
            self.widgets.iter_mut().map(|p| p.w.width(&Ctx { gfx, look, opts: &p.opts, h: row_h_dock, open: ex })).collect()
        } else {
            Vec::new()
        };
        let l = &self.layout;
        let scale = self.scale();
        let dock = self.dock();
        let bottom = self.bottom();
        let h_bar = l.geometry.expanded_height;
        // How much of the window shows: the deepest part of the shape, the corners below it.
        let deepest = frame.bumps.iter().map(|b| b.depth).fold(frame.base, f64::max).max(bumps.iter().map(|b| b.height).fold(0.0, f64::max));
        let shown_h = ((deepest + l.corner_radius + 2.0) * scale as f64).ceil() as i32;
        if shown_h > self.dib.h {
            self.dib = Dib::new(self.dib.w, shown_h + 64)?;
        }
        let h_px = shown_h.clamp(1, self.dib.h);
        let w_px = self.dib.w;
        let flip = if bottom { Some(h_px as f32 / scale) } else { None };
        // Edge distance → window y (logical).
        let y = |d: f64| -> f32 {
            match flip {
                Some(h) => h - d as f32,
                None => d as f32,
            }
        };

        gfx.begin(self.dib.dc, w_px, h_px, scale)?;
        let fill = with_alpha(self.look.bg, self.look.opacity);
        let shape = gfx.path(&frame.path, flip)?;
        unsafe {
            gfx.brush.SetColor(&fill);
            gfx.rt.FillGeometry(&shape, &gfx.brush, None::<&ID2D1Brush>);
            if !frame.corners.is_empty() {
                gfx.rt.FillGeometry(&gfx.path(&frame.corners, flip)?, &gfx.brush, None::<&ID2D1Brush>);
            }
            // Content is clipped by the shape, as the page's content is by its clip path.
            let params = D2D1_LAYER_PARAMETERS {
                contentBounds: D2D_RECT_F { left: -1e6, top: -1e6, right: 1e6, bottom: 1e6 },
                geometricMask: std::mem::ManuallyDrop::new(Some(shape.clone().into())),
                maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                maskTransform: Matrix3x2::identity(),
                opacity: 1.0,
                opacityBrush: std::mem::ManuallyDrop::new(None),
                layerOptions: D2D1_LAYER_OPTIONS_NONE,
            };
            gfx.rt.PushLayer(&params, None);
            std::mem::ManuallyDrop::into_inner(params.geometricMask);
        }

        // Open-only parts (media keys): the dock's opening, a bar's second half (--bar-fade).
        let bar_open = if dock { ex } else if ex >= 0.999 { 1.0 } else { clamp01((ex as f64 - 0.5) / 0.5) as f32 };
        let mut drawn: Vec<(usize, Option<(f32, f32, f32)>)> = Vec::new();

        // Notch content.
        for (i, &di) in order.iter().enumerate() {
            let d = &self.drops[di];
            let Some(n) = &d.notch else { continue };
            let bf = &frame.bumps[i];
            let row = bf.row_depth.round();
            let w = if dock { bumps[i].width } else { d.ws.as_ref().map_or(d.width, |s| s.value) };
            let slot_x = (bf.anchor_cx - w / 2.0) as f32;
            let shown = d.shown.as_ref().map_or(1.0, |s| clamp01(s.value * 1.5 - 0.5)) as f32;
            let fits = if dock { clamp01((bf.row_depth - 16.0) / 10.0) as f32 } else { 1.0 };
            // The notch's own content cross-fades with a pop-up growing out of it.
            let osd_here = self.osd_drop() == Some(di) && !self.osd_below(false);
            let alpha = shown * fits * if osd_here { 1.0 - clamp01(self.pop.osd.value * 1.8) as f32 } else { 1.0 };
            let cy = y(row / 2.0);
            // Docks lay the slot out at its current (morphing) size.
            let kids: Vec<usize> = d.slot.iter().copied().filter(|&wi| self.widgets[wi].width > 0.0 && !self.covered(d, wi) && !self.aside(di, wi)).collect();
            let gap = l.bar.gap as f32;
            let mut xoff = if dock {
                // A dock corner notch's curves grow with it: so does its content's inset.
                let (pl, pr) = self.slot_pads(n, bumps[i].scale);
                let cw: f32 = kids.iter().map(|&wi| dock_widths[wi]).sum::<f32>() + gap * (kids.len().max(1) as f32 - 1.0);
                pl + (w as f32 - pl - pr - cw) / 2.0
            } else {
                0.0
            };
            for &wi in &kids {
                let p = &self.widgets[wi];
                let mut x = if dock {
                    let x = slot_x + xoff;
                    xoff += dock_widths[wi] + gap;
                    x
                } else {
                    slot_x + p.slot_x
                };
                if !dock {
                    // Pinned widgets glide from the notch to their place in the bar.
                    if let Some(to) = p.bar_x {
                        let rest = d.xs.as_ref().map_or(d.x, |s| s.value) as f32;
                        x = lerp((rest + p.slot_x) as f64, to as f64, ex as f64) as f32;
                    }
                }
                let x = (x * scale).round() / scale;
                let cx = Ctx { gfx, look: &self.look, opts: &p.opts, h: if dock { row_h_dock } else { row as f32 }, open: bar_open };
                if alpha > 0.0 {
                    p.w.draw(&cx, x, cy, alpha);
                }
                drawn.push((wi, Some((x, cy, alpha))));
            }
        }

        // The pop-up's content: in the grown notch, or in its pill below the open bar.
        let o = clamp01(self.pop.osd.value) as f32;
        if let (Some(od), true) = (self.osd_drop(), o > 0.01) {
            if let Some(i) = order.iter().position(|&di| di == od) {
                let bf = &frame.bumps[i];
                let (ow, _) = self.osd_size();
                let inner = (ow - 2.0 * l.geometry.notch_corner_radius - 8.0) as f32;
                let (cx, cy) = if self.osd_below(false) {
                    ((bf.panel_x + ow / 2.0) as f32, y(h_bar + (bf.depth - h_bar) / 2.0))
                } else {
                    (bf.cx as f32, y(bf.row_depth.round() / 2.0))
                };
                let side = self.side_of(od);
                let nudge = l.widgets.get("notifications").and_then(|w| w.get("anim_nudge")).and_then(|v| v.as_bool()).unwrap_or(l.animation.notification_nudge);
                let accent = l.theme.accent_details;
                let look = self.look.clone();
                self.pop.draw(gfx, &look, accent, cx, cy, inner, side, (o * 1.8 - 0.5).clamp(0.0, 1.0), nudge);
            }
        }
        let k = clamp01(self.pop.peek.value) as f32;
        if k > 0.01 {
            if let Some(i) = order.iter().position(|&di| self.drops[di].key == "peek") {
                let bf = &frame.bumps[i];
                self.pop.draw_peek(gfx, &self.look, bf.cx as f32, y(bf.row_depth.round() / 2.0), (k * 1.8 - 0.5).clamp(0.0, 1.0) * (1.0 - ex));
            }
        }

        // The open bar's row: slides with the bar's edge, or waits and is uncovered.
        if !dock {
            let (offset, alpha) = match l.animation.bar_content.as_str() {
                "reveal" => (0.0, clamp01(ex as f64 * 5.0)),
                "fade" => (0.0, clamp01((ex as f64 - 0.35) / 0.55)),
                _ => (frame.base - h_bar, clamp01((ex as f64 - 0.25) / 0.6)),
            };
            let fade = if ex >= 0.999 { 1.0 } else { clamp01((ex as f64 - 0.5) / 0.5) as f32 };
            let cy = y(offset + h_bar / 2.0);
            for (wi, p) in self.widgets.iter().enumerate() {
                if p.notch.is_some() {
                    continue;
                }
                let Some(x) = p.bar_x else { continue };
                let a = alpha as f32 * if p.fades { fade } else { 1.0 };
                if a > 0.0 {
                    let cx = Ctx { gfx, look: &self.look, opts: &p.opts, h: h_bar as f32, open: 1.0 };
                    p.w.draw(&cx, x, cy, a);
                }
                drawn.push((wi, if a > 0.0 { Some((x, cy, a)) } else { None }));
            }
        }

        // Panels: their content waits in its open spot and the growing shape uncovers it.
        let mut panel_drawn: Vec<(usize, Option<(f32, f32)>)> = Vec::new();
        for (i, &di) in order.iter().enumerate() {
            let d = &self.drops[di];
            let Some(pi) = d.active else { continue };
            let p = d.spring.value;
            if p <= 0.01 {
                panel_drawn.push((pi, None));
                continue;
            }
            let bf = &frame.bumps[i];
            let pb = &self.panels[pi];
            let near = bumps[i].panel_top.unwrap_or(h_bar);
            let (pad_near, pad_far, pad_x) = pb.p.pad();
            let alpha = if l.animation.panel_content == "fade" { clamp01(p * 1.6 - 0.5) as f32 } else { 1.0 };
            let free = pb.w as f32 - pb.content.0 - 2.0 * pad_x - pb.shift;
            let left = bf.panel_x as f32
                + pad_x
                + pb.shift
                + match pb.align {
                    Align::Center => free / 2.0,
                    Align::Left => 0.0,
                    Align::Right => free,
                };
            let top = if bottom { y(near + pb.h) + pad_far } else { y(near) + pad_near };
            let (left, top) = ((left * scale).round() / scale, (top * scale).round() / scale);
            let cx = Ctx { gfx, look: &self.look, opts: &pb.opts, h: h_bar as f32, open: 1.0 };
            match pb.cascade.filter(|_| pb.opts.cascade) {
                // Cascade: each piece fades in and slides 8 px into place (from the bar's side)
                // a little after the one before.
                Some(t0) if Instant::now() < t0 + cascade_length(pb.p.pieces().len()) => {
                    let pieces = pb.p.pieces();
                    let pieces = if pieces.is_empty() { vec![(0.0, 0.0, pb.content.0, pb.content.1)] } else { pieces };
                    let step = (240.0 / pieces.len().max(1) as f32).min(25.0);
                    let t = Instant::now().saturating_duration_since(t0).as_secs_f32() * 1000.0;
                    let away = if bottom { -1.0 } else { 1.0 };
                    for (i, (px, py, pw, ph)) in pieces.iter().enumerate() {
                        let k = ((t - i as f32 * step) / 160.0).clamp(0.0, 1.0);
                        let e = 1.0 - (1.0 - k).powi(3);
                        if e <= 0.0 {
                            continue;
                        }
                        let dy = (1.0 - e) * -8.0 * away;
                        gfx.clip(super::gfx::rect(left + px - 2.0, top + py - 2.0 + dy, pw + 4.0, ph + 4.0));
                        pb.p.draw(&cx, left, top + dy, alpha * e);
                        gfx.unclip();
                    }
                }
                _ => pb.p.draw(&cx, left, top, alpha),
            }
            panel_drawn.push((pi, Some((left, top))));
        }
        unsafe {
            gfx.rt.PopLayer();
        }
        gfx.end()?;
        for (wi, at) in drawn {
            self.widgets[wi].drawn = at;
        }
        for (pi, at) in panel_drawn {
            self.panels[pi].drawn = at;
        }

        let pos = POINT { x: self.monitor.left, y: if bottom { self.monitor.bottom - h_px } else { self.monitor.top } };
        let pos = if self.offscreen { POINT { x: -32000, y: -32000 } } else { pos };
        let size = SIZE { cx: w_px, cy: h_px };
        let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8 };
        unsafe {
            UpdateLayeredWindow(self.hwnd, None, Some(&pos), Some(&size), Some(self.dib.dc), Some(&POINT { x: 0, y: 0 }), COLORREF(0), Some(&blend), ULW_ALPHA)?;
        }
        self.size = size;
        self.pos = pos;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Interaction.

    /// Interactive areas for the engine's click-through and hover-leave (main.ts sendRegions).
    fn update_regions(&mut self) {
        let l = &self.layout;
        let mut regions: Vec<Rect> = Vec::new();
        let order = self.ordered();
        if self.expanded && self.dock() {
            let (bumps, frame) = self.frame_of(&order, true);
            for (i, &di) in order.iter().enumerate() {
                if !self.drops[di].in_hull {
                    continue;
                }
                let bf = &frame.bumps[i];
                let r = l.geometry.concave_radius * bumps[i].scale;
                regions.push(Rect { x: bf.left - r, y: 0.0, width: bf.right - bf.left + 2.0 * r, height: bf.depth });
            }
        } else if self.expanded {
            let h = l.geometry.expanded_height;
            let r = l.geometry.panel_concave_radius;
            regions.push(Rect { x: 0.0, y: 0.0, width: l.width, height: h });
            let (_, frame) = self.frame_of(&order, true);
            for (i, &di) in order.iter().enumerate() {
                let d = &self.drops[di];
                if !d.has_panel || d.spring.target < 0.5 {
                    continue;
                }
                let bf = &frame.bumps[i];
                regions.push(Rect { x: bf.left - r, y: h - 1.0, width: bf.right - bf.left + 2.0 * r, height: bf.depth - h + 1.0 });
            }
        }
        let key = regions.iter().map(|r| format!("{:.1},{:.1},{:.1},{:.1}", r.x, r.y, r.width, r.height)).collect::<Vec<_>>().join(";");
        if key != self.sent_regions {
            self.sent_regions = key;
            self.pending_regions = Some(regions);
        }
    }

    pub fn take_widths(&mut self) -> Option<HashMap<String, NotchWidth>> {
        self.pending_widths.take()
    }

    pub fn take_regions(&mut self) -> Option<Vec<Rect>> {
        self.pending_regions.take()
    }

    /// Physical client px → logical window px.
    fn logical(&self, p: (i32, i32)) -> (f32, f32) {
        (p.0 as f32 / self.scale(), p.1 as f32 / self.scale())
    }

    /// The widget under a point (window px, logical), with the point in its own terms.
    fn widget_at(&self, gfx: &Gfx, x: f32, y: f32) -> Option<(usize, (f32, f32))> {
        let _ = gfx;
        self.widgets.iter().enumerate().find_map(|(wi, p)| {
            let (wx, cy, a) = p.drawn?;
            let half = (self.layout.geometry.expanded_height as f32).max(self.layout.geometry.notch_height as f32) / 2.0;
            (a > 0.05 && x >= wx && x <= wx + p.width.max(p.open_width) && (y - cy).abs() <= half).then_some((wi, (x - wx, y - cy)))
        })
    }

    /// The open panel under a point, with the point in its content's terms.
    fn panel_at(&self, x: f32, y: f32) -> Option<(usize, usize, (f32, f32))> {
        self.drops.iter().enumerate().find_map(|(di, d)| {
            let pi = d.active?;
            if d.spring.target < 0.5 {
                return None;
            }
            let pb = &self.panels[pi];
            let (cx, cy) = pb.drawn?;
            let (pad_near, pad_far, pad_x) = pb.p.pad();
            let pad_top = if self.bottom() { pad_far } else { pad_near };
            let (bx, by) = (cx - pad_x - pb.shift, cy - pad_top);
            (x >= bx && x <= bx + pb.w as f32 && y >= by && y <= by + pb.h as f32).then_some((di, pi, (x - cx, y - cy)))
        })
    }

    pub fn mouse_move(&mut self, gfx: &Gfx, at: Option<(i32, i32)>) {
        if at.is_some() && !self.tracking {
            self.tracking = true;
            let mut t = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: self.hwnd, dwHoverTime: 0 };
            unsafe {
                let _ = TrackMouseEvent(&mut t);
            }
        }
        if at.is_none() {
            self.tracking = false;
        }
        let pt = at.map(|p| self.logical(p));
        let mut changed = false;
        let over_panel = pt.and_then(|(x, y)| self.panel_at(x, y));
        for pi in 0..self.panels.len() {
            let local = over_panel.filter(|(_, p, _)| *p == pi).map(|(_, _, l)| l);
            changed |= self.panels[pi].p.hover(local);
        }
        let over_widget = if over_panel.is_none() { pt.and_then(|(x, y)| self.widget_at(gfx, x, y)) } else { None };
        for wi in 0..self.widgets.len() {
            let local = over_widget.filter(|(w, _)| *w == wi).map(|(_, l)| l);
            changed |= self.widgets[wi].w.hover(local);
        }
        if changed {
            // Hover effects that move (dock icons magnifying) draw frames until they settle.
            let now = Instant::now();
            if self.widgets.iter().any(|w| w.w.animating(now)) {
                self.kick();
            } else {
                self.redraw(gfx);
            }
        }
    }

    /// A click (released) at a physical client point. Returns an action to run.
    pub fn click(&mut self, gfx: &Gfx, button: Button, at: (i32, i32)) -> Option<Act> {
        let (x, y) = self.logical(at);
        let reply = if let Some((_, pi, local)) = self.panel_at(x, y) {
            // A tray click's point, local to the panel's content: on screen, in device px.
            let origin = self.panels[pi].drawn.unwrap_or((0.0, 0.0));
            let (pos, scale) = (self.pos, self.scale());
            let to_screen = move |at: (f32, f32)| (pos.x as f32 + (origin.0 + at.0) * scale, pos.y as f32 + (origin.1 + at.1) * scale);
            match self.panels[pi].p.click(button, local) {
                Reply::CloseAnd(Act::Tray { id, button, at }) => Reply::CloseAnd(Act::Tray { id, button, at: to_screen(at) }),
                other => other,
            }
        } else if let Some((wi, local)) = self.widget_at(gfx, x, y) {
            // A tray click's point, local to the widget: on screen, in device px.
            let origin = self.widgets[wi].drawn.map_or((0.0, 0.0), |(wx, cy, _)| (wx, cy));
            let (pos, scale) = (self.pos, self.scale());
            let to_screen = move |at: (f32, f32)| (pos.x as f32 + (origin.0 + at.0) * scale, pos.y as f32 + (origin.1 + at.1) * scale);
            let reply = match self.widgets[wi].w.click(button, local) {
                Reply::Do(Act::Tray { id, button, at }) => Reply::Do(Act::Tray { id, button, at: to_screen(at) }),
                other => other,
            };
            match reply {
                Reply::TogglePanel => {
                    self.toggle_panel_of(gfx, wi);
                    Reply::None
                }
                Reply::ToggleAnd(act) => {
                    self.toggle_panel_of(gfx, wi);
                    Reply::Do(act)
                }
                other => other,
            }
        } else {
            Reply::None
        };
        self.answer(gfx, reply)
    }

    /// What a reply asks of the surface; the action to run, if any.
    /// The launcher acts in a reply, made calls with this surface's label.
    fn with_label(&self, act: Act) -> Act {
        match act {
            Act::Launcher { open, refocus } => {
                let label = self.label.clone();
                Act::call(move |app| crate::engine::launcher_toggle(&app, Some(label), Some(open), refocus))
            }
            Act::Seq(list) => Act::Seq(list.into_iter().map(|a| self.with_label(a)).collect()),
            other => other,
        }
    }

    fn answer(&mut self, gfx: &Gfx, reply: Reply) -> Option<Act> {
        let reply = match reply {
            Reply::Do(a) => Reply::Do(self.with_label(a)),
            Reply::CloseAnd(a) => Reply::CloseAnd(self.with_label(a)),
            Reply::ToggleAnd(a) => Reply::ToggleAnd(self.with_label(a)),
            other => other,
        };
        if self.panels.iter().any(|p| p.p.animating(Instant::now())) {
            self.kick();
        }
        match reply {
            Reply::None | Reply::TogglePanel => None,
            Reply::Redraw => {
                self.remeasure = true;
                self.measure(gfx);
                self.kick();
                None
            }
            Reply::ToggleAnd(act) | Reply::Do(act) => Some(act),
            Reply::CloseAnd(act) => {
                self.close_all();
                Some(act)
            }
        }
    }

    /// The left button down: a panel part that drags (a slider) takes the mouse.
    pub fn press(&mut self, gfx: &Gfx, at: (i32, i32)) -> bool {
        let (x, y) = self.logical(at);
        let Some((_, pi, local)) = self.panel_at(x, y) else { return false };
        if !self.panels[pi].p.press(local) {
            return false;
        }
        self.captured = Some(pi);
        unsafe {
            SetCapture(self.hwnd);
        }
        self.redraw(gfx);
        true
    }

    /// The panel part being dragged (local to its content), if any.
    fn captured_local(&self, at: (i32, i32)) -> Option<(usize, (f32, f32))> {
        let pi = self.captured?;
        let (x, y) = self.logical(at);
        let (cx, cy) = self.panels[pi].drawn?;
        Some((pi, (x - cx, y - cy)))
    }

    pub fn dragging(&self) -> bool {
        self.captured.is_some()
    }

    pub fn drag(&mut self, gfx: &Gfx, at: (i32, i32)) -> Option<Act> {
        let (pi, local) = self.captured_local(at)?;
        let reply = self.panels[pi].p.drag(local);
        let act = self.answer(gfx, reply);
        self.redraw(gfx);
        act
    }

    pub fn release(&mut self, gfx: &Gfx, at: (i32, i32)) -> Option<Act> {
        let (pi, local) = self.captured_local(at)?;
        self.captured = None;
        unsafe {
            let _ = ReleaseCapture();
        }
        let reply = self.panels[pi].p.release(local);
        let act = self.answer(gfx, reply);
        self.redraw(gfx);
        act
    }

    /// The wheel over a panel (sliders step with it).
    pub fn wheel(&mut self, gfx: &Gfx, at: (i32, i32), notches: f32) -> Option<Act> {
        let (x, y) = self.logical(at);
        let (_, pi, local) = self.panel_at(x, y)?;
        let reply = self.panels[pi].p.wheel(local, notches);
        let act = self.answer(gfx, reply);
        self.redraw(gfx);
        act
    }

    /// A widget's panel: its bar dropdown, or (docks) its notch's panel.
    fn toggle_panel_of(&mut self, gfx: &Gfx, wi: usize) {
        self.panel_of(gfx, wi, None);
    }

    /// Opens (Some(true)), closes or toggles a widget's panel.
    fn panel_of(&mut self, gfx: &Gfx, wi: usize, force: Option<bool>) {
        if !self.expanded {
            return;
        }
        if let Some(di) = self.drops.iter().position(|d| d.anchor == Some(wi)) {
            let open = force.unwrap_or(self.drops[di].spring.target < 0.5);
            if open {
                self.remeasure = true;
                self.measure(gfx);
            }
            for (i, d) in self.drops.iter_mut().enumerate() {
                d.spring.target = if i == di && open { 1.0 } else { 0.0 };
            }
        } else if let Some(di) = self.drops.iter().position(|d| d.panels.iter().any(|(w, _)| *w == wi)) {
            let pi = self.drops[di].panels.iter().find(|(w, _)| *w == wi).map(|p| p.1);
            let open = force.unwrap_or(!(self.drops[di].active == pi && self.drops[di].spring.target > 0.5));
            for (i, d) in self.drops.iter_mut().enumerate() {
                if i != di {
                    d.spring.target = 0.0;
                }
            }
            if open {
                self.drops[di].active = pi;
                self.remeasure = true;
                self.measure(gfx);
            }
            self.drops[di].spring.target = if open { 1.0 } else { 0.0 };
        } else {
            return;
        }
        self.kick();
        self.update_regions();
    }

    fn close_all(&mut self) {
        for d in &mut self.drops {
            d.spring.target = 0.0;
        }
        self.kick();
        self.update_regions();
    }

    // ------------------------------------------------------------------
    // Snapshots (native/snapshot.rs): set a state and draw it at once.

    /// Snaps everything to a state: bar / dock open or not, one panel (by widget name) open.
    pub fn pose(&mut self, gfx: &Gfx, expanded: bool, panel: Option<&str>, p: f64) {
        self.set_expanded(gfx, expanded);
        for d in &mut self.drops {
            d.spring.target = 0.0;
        }
        if let Some(name) = panel {
            if let Some(wi) = self.widgets.iter().position(|w| w.name == name) {
                self.panel_of(gfx, wi, Some(true));
            }
        }
        let springs = [&mut self.expand, &mut self.spread, &mut self.nudge];
        for s in springs {
            s.snap();
        }
        for d in &mut self.drops {
            d.spring.snap();
            if d.spring.target > 0.5 {
                d.spring.value = p;
            }
            for s in [d.xs.as_mut(), d.ws.as_mut(), d.shown.as_mut()].into_iter().flatten() {
                s.snap();
            }
        }
        self.pop.osd.snap();
        self.pop.peek.snap();
        // Posed open: any cascade has long finished.
        for pb in &mut self.panels {
            pb.cascade = Some(Instant::now() - std::time::Duration::from_secs(10));
        }
        self.animating = false;
    }

    /// The window's pixels as last drawn (BGRA, premultiplied) and their size.
    pub fn pixels(&self) -> (&[u8], i32, i32) {
        let (w, h) = (self.size.cx, self.size.cy);
        (unsafe { std::slice::from_raw_parts(self.dib.bits, (self.dib.w * h * 4) as usize) }, w, h)
    }

    pub fn widget_names(&self) -> Vec<String> {
        self.widgets.iter().map(|w| w.name.clone()).collect()
    }
}
