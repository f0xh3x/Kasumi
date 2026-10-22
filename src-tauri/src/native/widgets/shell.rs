//! Media (src/widgets/media.ts), the quick settings button (quick.ts) and the
//! tray (tray.ts): the chevron that drops the tray panel, or every icon inline.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use serde_json::Value;

use super::icon::{decode, AppIcon, Pixels, Style};
use super::{fit_text, row_draw, row_width, Act, Button, Ctx, Opts, Piece, Reply, Roll, Widget};
use crate::native::gfx::{rect, white, with_alpha};

/// One clock for every scrolling title (the pages' document clock): the notch's
/// and the bar's copies scroll in step.
fn epoch() -> Instant {
    static T: OnceLock<Instant> = OnceLock::new();
    *T.get_or_init(Instant::now)
}

const PREV: &str = "\u{E892}";
const NEXT: &str = "\u{E893}";
const PLAY: &str = "\u{E768}";
const PAUSE: &str = "\u{E769}";

/// Now playing: previous / play-pause / next, then "title — artist".
pub struct Media {
    opts: Opts,
    line: Option<String>,
    /// Title, and the artist under it.
    tip: String,
    playing: bool,
    can: [bool; 3],
    hovered: Option<usize>,
    /// Where the keys start (local), the title's width and its box's, as last measured.
    keys_x: f32,
    label_w: f32,
    box_w: f32,
}

// .media-btn: 24 × 22, 2 px apart, 12 px icons; the title 2 px from the keys, 6 px gap.
const KEY_W: f32 = 24.0;
const KEY_H: f32 = 22.0;
const KEYS_W: f32 = 3.0 * KEY_W + 2.0 * 2.0;

impl Media {
    pub fn new(opts: Opts) -> Media {
        Media { opts, line: None, tip: String::new(), playing: false, can: [false; 3], hovered: None, keys_x: 0.0, label_w: 0.0, box_w: 0.0 }
    }

    fn keys(&self) -> &str {
        match self.opts.table.get("keys") {
            Some(toml::Value::Boolean(true)) => "always",
            Some(toml::Value::String(s)) => s.as_str(),
            _ => "off",
        }
    }

    /// How much the keys show (0 hidden, 1 shown) at this openness.
    fn keys_shown(&self, open: f32) -> f32 {
        match self.keys() {
            "always" => 1.0,
            "open" => open,
            "closed" if open <= 0.0 => 1.0,
            _ => 0.0,
        }
    }

    fn max_w(&self) -> f32 {
        self.opts.num("max_width").unwrap_or(240.0) as f32
    }

    /// The title box's width.
    fn text_w(&self, cx: &Ctx) -> f32 {
        let Some(line) = &self.line else { return 0.0 };
        if self.opts.flag("fixed_width") == Some(true) {
            return self.max_w();
        }
        cx.gfx.measure(line, &cx.body()).min(self.max_w()).ceil()
    }

    /// The marquee: (scrolling, offset now, when it next moves a pixel).
    fn scroll(&self, now: Instant) -> (bool, f32, Option<Instant>) {
        if self.line.is_none() || self.opts.flag("scroll") != Some(true) || self.label_w <= self.box_w + 1.0 {
            return (false, 0.0, None);
        }
        let speed = self.opts.num("scroll_speed").unwrap_or(32.0).clamp(5.0, 120.0) as f32;
        let (hold, pixel) = (1800.0, 1000.0 / speed);
        let total = hold + (self.label_w + 36.0) * pixel;
        let t = (now.saturating_duration_since(epoch()).as_secs_f64() * 1000.0) as f32 % total;
        let x = if t < hold { 0.0 } else { ((t - hold) / pixel).floor() };
        let wait = if t < hold { hold - t } else { pixel - ((t - hold) % pixel) };
        (true, x, Some(now + Duration::from_secs_f32(wait.max(1.0) / 1000.0)))
    }
}

impl Widget for Media {
    fn keys(&self) -> &'static [&'static str] {
        &["media"]
    }
    fn tooltip(&self) -> Option<String> {
        // Not over the keys: those say what they do by their icons.
        (self.line.is_some() && self.hovered.is_none()).then(|| self.tip.clone()).filter(|t| !t.is_empty())
    }
    fn data(&mut self, _key: &str, v: &Value) -> bool {
        if v.is_null() {
            let had = self.line.is_some();
            self.line = None;
            return had;
        }
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
        let b = |k: &str| v.get(k).and_then(|x| x.as_bool()).unwrap_or(false);
        let (title, artist) = (s("title"), s("artist"));
        self.tip = if artist.is_empty() { title.clone() } else { format!("{title}\n{artist}") };
        let line = Some(if artist.is_empty() { title } else { format!("{title} — {artist}") });
        let can = [b("canPrevious"), b("canPlayPause"), b("canNext")];
        let changed = line != self.line || b("playing") != self.playing || can != self.can;
        self.line = line;
        self.playing = b("playing");
        self.can = can;
        changed
    }
    fn wake(&self) -> Option<Instant> {
        // The title scrolls a whole pixel at a time: a redraw per pixel, not per display refresh.
        self.scroll(Instant::now()).2
    }
    fn tick(&mut self, _now: Instant) -> bool {
        true
    }
    fn tick_resizes(&self) -> bool {
        false
    }
    fn width(&mut self, cx: &Ctx) -> f32 {
        let Some(line) = &self.line else { return 0.0 };
        self.label_w = cx.gfx.measure(line, &cx.body());
        self.box_w = self.text_w(cx);
        let keys = if self.keys_shown(cx.open) > 0.0 { KEYS_W + 6.0 + 2.0 } else { 0.0 };
        self.keys_x = if self.opts.str("keys_side") == Some("right") { self.box_w + 2.0 + 6.0 } else { 0.0 };
        keys + self.box_w
    }
    fn draw(&self, cx: &Ctx, x: f32, cy: f32, alpha: f32) {
        let Some(line) = &self.line else { return };
        let g = cx.gfx;
        let shown = self.keys_shown(cx.open);
        let right = self.opts.str("keys_side") == Some("right");
        let tw = self.text_w(cx);
        let (keys_x, text_x) = match (shown > 0.0, right) {
            (false, _) => (None, x),
            (true, false) => (Some(x), x + KEYS_W + 6.0 + 2.0),
            (true, true) => (Some(x + tw + 2.0 + 6.0), x),
        };
        if let Some(kx) = keys_x {
            let glyphs = [PREV, if self.playing { PAUSE } else { PLAY }, NEXT];
            for (i, glyph) in glyphs.iter().enumerate() {
                let bx = kx + i as f32 * (KEY_W + 2.0);
                let on = if self.can[i] { 1.0 } else { 0.35 };
                if self.hovered == Some(i) && self.can[i] {
                    g.fill_round(rect(bx, cy - KEY_H / 2.0, KEY_W, KEY_H), 6.0, with_alpha(cx.hover_bg(0.12), alpha * shown));
                }
                let f = crate::native::gfx::Font::icons(12.0);
                let w = g.measure(glyph, &f);
                g.text_at(glyph, &f, bx + (KEY_W - w) / 2.0, cy, with_alpha(cx.look.fg, alpha * shown * on));
            }
        }
        let body = cx.body();
        let color = white(0.85 * alpha);
        let (scrolling, off, _) = self.scroll(Instant::now());
        if scrolling {
            let line_h = g.text(line, &body).map_or(16.0, |t| t.height);
            g.push_fade(rect(text_x, cy - line_h, tw, line_h * 2.0), 6.0, 14.0);
            let label = g.measure(line, &body);
            g.text_at(line, &body, text_x - off, cy, color);
            g.text_at(line, &body, text_x - off + label + 36.0, cy, color);
            g.pop_fade();
            return;
        }
        let shown_line = fit_text(g, line, &body, tw);
        let w = g.measure(&shown_line, &body);
        // A fixed-width title sits in its box as title_align says (centre by default).
        let align = if self.opts.flag("fixed_width") == Some(true) { self.opts.str("title_align").unwrap_or("center") } else { "left" };
        let lx = match align {
            "center" => text_x + (tw - w) / 2.0,
            "right" => text_x + tw - w,
            _ => text_x,
        };
        g.text_at(&shown_line, &body, lx, cy, color);
    }
    fn animating(&self, _now: Instant) -> bool {
        false
    }
    fn hover(&mut self, at: Option<(f32, f32)>) -> bool {
        let h = at.and_then(|(x, _)| self.key_at(x));
        let changed = h != self.hovered;
        self.hovered = h;
        changed
    }
    fn click(&mut self, button: Button, at: (f32, f32)) -> Reply {
        if button != Button::Left {
            return Reply::None;
        }
        match self.key_at(at.0) {
            Some(i) if self.can[i] => {
                let action = ["previous", "play_pause", "next"][i];
                Reply::Do(Act::call(move |app| {
                    use tauri::Manager;
                    let sender = app.state::<crate::providers::media::MediaSender>();
                    let _ = tauri::async_runtime::block_on(crate::providers::media::media_control(sender, action.into(), None));
                }))
            }
            Some(_) => Reply::None,
            None if button == Button::Left => Reply::TogglePanel,
            None => Reply::None,
        }
    }
}

impl Media {
    /// The key under a local x.
    fn key_at(&self, x: f32) -> Option<usize> {
        if self.keys_shown(1.0) <= 0.0 {
            return None;
        }
        let left = self.keys_x;
        let i = ((x - left) / (KEY_W + 2.0)).floor();
        (i >= 0.0 && i < 3.0 && (x - left) - i * (KEY_W + 2.0) < KEY_W).then_some(i as usize)
    }
}

// ----------------------------------------------------------------------

/// The battery's glyph at a level (charging or not).
pub fn battery_glyph(percent: f64, charging: bool) -> String {
    let step = (percent / 10.0).round().clamp(0.0, 10.0) as u32;
    let code = if charging {
        match step {
            10 => 0xEA93,
            9 => 0xE83E,
            s => 0xE85A + s,
        }
    } else if step >= 10 {
        0xE83F
    } else {
        0xE850 + step
    };
    char::from_u32(code).map(String::from).unwrap_or_default()
}

pub fn volume_glyph(level: f64, muted: bool) -> &'static str {
    if muted {
        "\u{E74F}"
    } else if level == 0.0 {
        "\u{E992}"
    } else if level < 0.34 {
        "\u{E993}"
    } else if level < 0.67 {
        "\u{E994}"
    } else {
        "\u{E995}"
    }
}

pub fn network_glyph(n: &Value) -> &'static str {
    if !n.get("connected").and_then(|c| c.as_bool()).unwrap_or(false) {
        "\u{F384}"
    } else if n.get("wifi").and_then(|w| w.as_bool()).unwrap_or(false) {
        "\u{E701}"
    } else {
        "\u{E839}"
    }
}

/// The quick settings button: battery (on PCs with one) or a gear (`icon`), its panel the settings.
pub struct Quick {
    opts: Opts,
    tip: String,
    glyph: String,
    value: Roll,
}

impl Quick {
    pub fn new(opts: Opts) -> Quick {
        Quick { opts, tip: "Quick settings".into(), glyph: "\u{E713}".into(), value: Roll::default() }
    }
    fn pieces(&self, cx: &Ctx) -> Vec<Piece> {
        let mut out = vec![Piece::glyph(self.glyph.clone(), 14.0, cx.look.fg)];
        if !self.value.text.is_empty() {
            out.push(Piece::Gap(5.0));
            out.push(self.value.piece(cx.body(), cx.look.fg));
        }
        out
    }
}

impl Widget for Quick {
    fn keys(&self) -> &'static [&'static str] {
        &["system"]
    }
    fn tooltip(&self) -> Option<String> {
        Some(self.tip.clone())
    }
    fn data(&mut self, _key: &str, s: &Value) -> bool {
        let mode = self.opts.str("icon").unwrap_or("auto");
        let battery = s.get("battery").filter(|b| !b.is_null());
        let show = if mode == "auto" { if battery.is_some() { "battery" } else { "settings" } } else { mode };
        let mut text = String::new();
        let glyph = match show {
            "battery" if battery.and_then(|b| b.get("percent")).and_then(|p| p.as_f64()).is_some() => {
                let b = battery.unwrap();
                let p = b.get("percent").and_then(|p| p.as_f64()).unwrap_or(0.0);
                text = format!("{p}%");
                battery_glyph(p, b.get("charging").and_then(|c| c.as_bool()).unwrap_or(false))
            }
            "volume" if s.get("volume").is_some_and(|v| !v.is_null()) => {
                let v = &s["volume"];
                volume_glyph(v.get("level").and_then(|l| l.as_f64()).unwrap_or(0.0), v.get("muted").and_then(|m| m.as_bool()).unwrap_or(false)).into()
            }
            "network" => network_glyph(s.get("network").unwrap_or(&Value::Null)).into(),
            "custom" if self.opts.str("custom_icon").is_some_and(|c| !c.is_empty()) => self.opts.str("custom_icon").unwrap().into(),
            _ => "\u{E713}".into(),
        };
        let net = s.get("network").filter(|n| n.get("connected").and_then(|c| c.as_bool()).unwrap_or(false));
        self.tip = match show {
            "battery" => battery.and_then(|b| {
                let p = b.get("percent")?.as_f64()?;
                Some(format!("Battery {p}%{}", if b.get("charging").and_then(|c| c.as_bool()).unwrap_or(false) { ", charging" } else { "" }))
            }),
            "network" => Some(net.and_then(|n| n.get("name")?.as_str().map(String::from)).unwrap_or_else(|| "Not connected".into())),
            _ => None,
        }
        .unwrap_or_else(|| "Quick settings".into());
        let changed = glyph != self.glyph;
        self.glyph = glyph;
        self.value.set(text, self.opts.roll) || changed
    }
    fn width(&mut self, cx: &Ctx) -> f32 {
        row_width(cx.gfx, &self.pieces(cx))
    }
    fn draw(&self, cx: &Ctx, x: f32, cy: f32, alpha: f32) {
        row_draw(cx.gfx, &self.pieces(cx), x, cy, alpha);
    }
    fn animating(&self, now: Instant) -> bool {
        self.value.animating(now)
    }
}

// ----------------------------------------------------------------------

struct TrayIcon {
    id: String,
    tip: String,
    rgba: String,
    px: Option<Pixels>,
}

/// The tray: a chevron dropping the tray panel (`mode` = "collapsed"), or every
/// hosted icon right in the bar ("expanded"), clicks going to their apps.
pub struct Tray {
    opts: Opts,
    icons: Vec<TrayIcon>,
    bitmaps: HashMap<String, (String, AppIcon)>,
    style: Option<Style>,
    hovered: Option<usize>,
    h: f32,
}

impl Tray {
    pub fn new(opts: Opts) -> Tray {
        Tray { opts, icons: Vec::new(), bitmaps: HashMap::new(), style: None, hovered: None, h: 30.0 }
    }
    fn inline(&self) -> bool {
        self.opts.str("mode") == Some("expanded")
    }
    /// Inline cells (.tray-inline): 0.8 h square, 0.06 h apart.
    fn cell(&self) -> (f32, f32) {
        (self.h * 0.8, self.h * 0.06)
    }
    fn at(&self, x: f32) -> Option<usize> {
        let (side, gap) = self.cell();
        let i = (x / (side + gap)).floor();
        (i >= 0.0 && (i as usize) < self.icons.len() && x - i * (side + gap) < side).then_some(i as usize)
    }
}

impl Widget for Tray {
    fn keys(&self) -> &'static [&'static str] {
        &["tray"]
    }
    fn tooltip(&self) -> Option<String> {
        if !self.inline() {
            let n = self.icons.len();
            return Some(if n == 0 { "No tray icons".into() } else { format!("{n} tray icon{}", if n == 1 { "" } else { "s" }) });
        }
        self.icons.get(self.hovered?).map(|i| i.tip.clone()).filter(|t| !t.is_empty())
    }
    fn data(&mut self, _key: &str, v: &Value) -> bool {
        let list = v.as_array().cloned().unwrap_or_default();
        self.icons = list
            .iter()
            .filter_map(|i| {
                let id = i.get("id")?.as_str()?.to_string();
                let rgba = i.get("rgba")?.as_str()?.to_string();
                let fresh = !self.bitmaps.get(&id).is_some_and(|(r, _)| *r == rgba);
                let s = |k: &str| i.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
                let tip = Some(s("tip")).filter(|t| !t.is_empty()).unwrap_or_else(|| s("exe"));
                Some(TrayIcon { id, tip, px: if fresh { decode(i) } else { None }, rgba })
            })
            .collect();
        true
    }
    fn width(&mut self, cx: &Ctx) -> f32 {
        if !self.inline() {
            return cx.gfx.measure("\u{E70D}", &crate::native::gfx::Font::icons(14.0));
        }
        self.h = cx.h;
        let style = Style::from(self.opts.str("icons"), cx.look.accent);
        if self.style != Some(style) {
            self.style = Some(style);
            self.bitmaps.clear();
        }
        for i in &mut self.icons {
            if let Some(px) = i.px.take() {
                if let Some(b) = AppIcon::new(cx.gfx, &px, style) {
                    self.bitmaps.insert(i.id.clone(), (i.rgba.clone(), b));
                }
            }
        }
        let (side, gap) = self.cell();
        let n = self.icons.len() as f32;
        if n == 0.0 { 0.0 } else { n * side + (n - 1.0) * gap }
    }
    fn draw(&self, cx: &Ctx, x: f32, cy: f32, alpha: f32) {
        let g = cx.gfx;
        if !self.inline() {
            g.text_at("\u{E70D}", &crate::native::gfx::Font::icons(14.0), x, cy + 1.0, with_alpha(cx.look.fg, alpha));
            return;
        }
        let (side, gap) = self.cell();
        let img = self.h * 0.55;
        for (i, icon) in self.icons.iter().enumerate() {
            let bx = x + i as f32 * (side + gap);
            if self.hovered == Some(i) {
                g.fill_round(rect(bx, cy - side / 2.0, side, side), 8.0, with_alpha(cx.hover_bg(0.12), alpha));
            }
            if let Some((_, b)) = self.bitmaps.get(&icon.id) {
                b.draw(g, g.snap(bx + (side - img) / 2.0), g.snap(cy - img / 2.0), img, alpha);
            }
        }
    }
    fn hover(&mut self, at: Option<(f32, f32)>) -> bool {
        if !self.inline() {
            return false;
        }
        let h = at.and_then(|(x, _)| self.at(x));
        let changed = h != self.hovered;
        self.hovered = h;
        changed
    }
    fn click(&mut self, button: Button, at: (f32, f32)) -> Reply {
        if !self.inline() {
            return if button == Button::Left { Reply::TogglePanel } else { Reply::None };
        }
        let Some(i) = self.at(at.0) else { return Reply::None };
        let (side, gap) = self.cell();
        let centre = (i as f32 * (side + gap) + side / 2.0, 0.0);
        let button = match button {
            Button::Left => "left",
            Button::Right => "right",
            Button::Middle => return Reply::None,
        };
        Reply::Do(Act::Tray { id: self.icons[i].id.clone(), button, at: centre })
    }
}
