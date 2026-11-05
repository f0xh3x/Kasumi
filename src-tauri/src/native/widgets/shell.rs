//! Media (src/widgets/media.ts), the quick settings button (quick.ts) and the
//! tray (tray.ts): the chevron that drops the tray panel, or every icon inline.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use serde_json::Value;

use super::icon::{decode, AppIcon, Pixels, Style};
use super::panels2::{art_bytes, RECORD_TURN};
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

/// What shows before the title ([widgets.media] mark), whatever the panel's look.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Mark {
    None,
    /// The cover as a little record, turning while it plays.
    Record,
    /// Bars rising and falling while it plays.
    Bars,
    /// The cover, small.
    Cover,
}

/// Now playing: previous / play-pause / next, then "title — artist"; with [widgets.media]
/// mark, a turning record, moving bars or the cover before the title.
pub struct Media {
    opts: Opts,
    mark: Mark,
    /// The cover (Record, Cover): as sent, and drawable.
    art: Option<String>,
    picture: Option<windows::Win32::Graphics::Direct2D::ID2D1Bitmap>,
    picture_for: Option<String>,
    /// The record's turn (radians), when it last turned, and when the mark moves next
    /// (Record, Glow while playing).
    angle: f32,
    turned: Instant,
    beat: Option<Instant>,
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
    /// When the title next moves a pixel, as scheduled (worked out again at that very
    /// moment it would already be the pixel after, and the tick never came due).
    due: std::cell::Cell<Option<Instant>>,
}

// .media-btn: 24 × 22, 2 px apart, 12 px icons; the title 2 px from the keys, 6 px gap.
const KEY_W: f32 = 24.0;
const KEY_H: f32 = 22.0;
const KEYS_W: f32 = 3.0 * KEY_W + 2.0 * 2.0;
/// The look's mark before the title, and the gap after it.
const MARK_W: f32 = 16.0;
const MARK_GAP: f32 = 8.0;

impl Media {
    pub fn new(opts: Opts) -> Media {
        let mark = match opts.str("mark") {
            Some("record") => Mark::Record,
            Some("bars") => Mark::Bars,
            Some("cover") => Mark::Cover,
            _ => Mark::None,
        };
        Media { mark, opts, art: None, picture: None, picture_for: None, angle: 0.0, turned: Instant::now(), beat: None, line: None, tip: String::new(), playing: false, can: [false; 3], hovered: None, keys_x: 0.0, label_w: 0.0, box_w: 0.0, due: std::cell::Cell::new(None) }
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

    /// The mark's width with its gap (none without one).
    fn mark_w(&self) -> f32 {
        if self.mark == Mark::None { 0.0 } else { MARK_W + MARK_GAP }
    }

    /// The mark moves (a turning record, moving bars) while something plays.
    fn moving(&self) -> bool {
        self.playing && matches!(self.mark, Mark::Record | Mark::Bars)
    }

    /// The look's mark, `x` its left, `cy` the row's centre.
    fn draw_mark(&self, cx: &Ctx, x: f32, cy: f32, alpha: f32) {
        let g = cx.gfx;
        let fill = if cx.opts.accent { cx.look.accent } else { cx.look.fg };
        let r = MARK_W / 2.0;
        match self.mark {
            Mark::Record => {
                match &self.picture {
                    Some(p) => g.fill_disc(p, x + r, cy, r, self.angle, alpha),
                    None => g.fill_round(rect(x, cy - r, MARK_W, MARK_W), r, white(0.3 * alpha)),
                }
                g.fill_round(rect(x + r - 2.5, cy - 2.5, 5.0, 5.0), 2.5, with_alpha(cx.look.bg, alpha));
            }
            Mark::Bars => {
                // Four bars, each rising and falling at its own pace (low and still when paused).
                let t = epoch().elapsed().as_secs_f32();
                for (i, (speed, phase)) in [(7.0f32, 0.0f32), (9.3, 1.7), (6.1, 3.1), (8.2, 4.4)].iter().enumerate() {
                    let level = if self.playing { 0.3 + 0.7 * (t * speed + phase).sin().abs() } else { 0.3 };
                    let h = (12.0 * level).round().max(2.0);
                    g.fill_round(rect(x + 1.0 + i as f32 * 3.75, cy + 6.0 - h, 2.5, h), 1.0, with_alpha(fill, alpha));
                }
            }
            Mark::Cover => match &self.picture {
                Some(p) => g.fill_picture(p, rect(x - 1.0, cy - 9.0, 18.0, 18.0), 5.0, alpha),
                None => g.fill_round(rect(x - 1.0, cy - 9.0, 18.0, 18.0), 5.0, white(0.2 * alpha)),
            },
            Mark::None => {}
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
        let art = v.get("art").and_then(|a| a.as_str()).map(String::from);
        let changed = line != self.line || b("playing") != self.playing || can != self.can || art != self.art;
        self.line = line;
        self.playing = b("playing");
        self.can = can;
        self.art = art;
        if self.moving() && self.beat.is_none() {
            self.turned = Instant::now();
            self.beat = Some(Instant::now());
        }
        changed
    }
    fn wake(&self) -> Option<Instant> {
        // The title scrolls a whole pixel at a time: a redraw per pixel, not per display
        // refresh. The mark moves about twelve times a second.
        let scroll = match self.due.get() {
            Some(due) => Some(due),
            None => {
                let due = self.scroll(Instant::now()).2;
                self.due.set(due);
                due
            }
        };
        [scroll, self.beat].into_iter().flatten().min()
    }
    fn tick(&mut self, now: Instant) -> bool {
        self.due.set(None);
        if self.moving() {
            self.angle = (self.angle + now.saturating_duration_since(self.turned).as_secs_f32().min(0.2) * RECORD_TURN) % std::f32::consts::TAU;
            if self.beat.is_some_and(|b| now >= b) {
                self.beat = Some(now + Duration::from_millis(83));
            }
        } else {
            self.beat = None;
        }
        self.turned = now;
        true
    }
    fn tick_resizes(&self) -> bool {
        false
    }
    fn width(&mut self, cx: &Ctx) -> f32 {
        if self.picture_for != self.art && matches!(self.mark, Mark::Record | Mark::Cover) {
            self.picture_for = self.art.clone();
            self.picture = self.art.as_deref().and_then(art_bytes).and_then(|b| cx.gfx.picture(&b).ok());
        }
        let Some(line) = &self.line else { return 0.0 };
        self.label_w = cx.gfx.measure(line, &cx.body());
        self.box_w = self.text_w(cx);
        let keys = if self.keys_shown(cx.open) > 0.0 { KEYS_W + 6.0 + 2.0 } else { 0.0 };
        self.keys_x = if self.opts.str("keys_side") == Some("right") { self.mark_w() + self.box_w + 2.0 + 6.0 } else { 0.0 };
        keys + self.mark_w() + self.box_w
    }
    fn draw(&self, cx: &Ctx, x: f32, cy: f32, alpha: f32) {
        let Some(line) = &self.line else { return };
        let g = cx.gfx;
        let shown = self.keys_shown(cx.open);
        let right = self.opts.str("keys_side") == Some("right");
        let tw = self.text_w(cx);
        let mark = self.mark_w();
        let (keys_x, mark_x) = match (shown > 0.0, right) {
            (false, _) => (None, x),
            (true, false) => (Some(x), x + KEYS_W + 6.0 + 2.0),
            (true, true) => (Some(x + mark + tw + 2.0 + 6.0), x),
        };
        if mark > 0.0 {
            self.draw_mark(cx, mark_x, cy, alpha);
        }
        let text_x = mark_x + mark;
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
    /// `battery_style = "bar"`: the battery's level as a bar instead of its percentage
    /// (and low: 20% or less, not charging).
    bar: Option<(f32, bool)>,
}

/// The battery bar's length (px).
const BATTERY_BAR: f32 = 24.0;

impl Quick {
    pub fn new(opts: Opts) -> Quick {
        Quick { opts, tip: "Quick settings".into(), glyph: "\u{E713}".into(), value: Roll::default(), bar: None }
    }
    fn pieces(&self, cx: &Ctx) -> Vec<Piece> {
        let mut out = vec![Piece::glyph(self.glyph.clone(), 14.0, cx.look.fg)];
        if let Some((v, low)) = self.bar {
            out.push(Piece::Gap(5.0));
            out.push(Piece::Bar { len: BATTERY_BAR, v, fill: super::basic::meter_fill(cx, low) });
        } else if !self.value.text.is_empty() {
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
        let mut bar = None;
        let glyph = match show {
            "battery" if battery.and_then(|b| b.get("percent")).and_then(|p| p.as_f64()).is_some() => {
                let b = battery.unwrap();
                let p = b.get("percent").and_then(|p| p.as_f64()).unwrap_or(0.0);
                let charging = b.get("charging").and_then(|c| c.as_bool()).unwrap_or(false);
                if self.opts.str("battery_style") == Some("bar") {
                    bar = Some(((p / 100.0) as f32, p <= 20.0 && !charging));
                } else {
                    text = format!("{p}%");
                }
                battery_glyph(p, charging)
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
        let changed = glyph != self.glyph || bar != self.bar;
        self.glyph = glyph;
        self.bar = bar;
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
    /// Its app's name and program file ("steam.exe").
    name: String,
    exe: String,
    rgba: String,
    px: Option<Pixels>,
}

/// Whether a tray icon's program is one of `pinned` (names, ".exe" optional, any case).
pub(super) fn tray_pinned(pinned: &[String], exe: &str) -> bool {
    let exe = exe.trim_end_matches(".exe").to_lowercase();
    pinned.iter().any(|p| p.trim().trim_end_matches(".exe").to_lowercase() == exe)
}

/// `[widgets.tray] pinned`: programs whose icons stay in the bar.
pub(super) fn tray_pins(opts: &Opts) -> Vec<String> {
    opts.table.get("pinned").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default()
}

/// What's under the cursor in the tray's row.
#[derive(Clone, Copy, PartialEq)]
enum TrayHit {
    Icon(usize),
    Chevron,
}

/// One thing in the tray's row: what, where (from its left), how wide, how faded in.
struct TrayCell {
    hit: TrayHit,
    x: f32,
    w: f32,
    alpha: f32,
}

/// The icons coming out or going back in (style "slide").
const TRAY_SLIDE: Duration = Duration::from_millis(220);
/// The cursor gone this long: slid-out icons go back in.
const TRAY_LINGER: Duration = Duration::from_millis(1200);

/// The tray: a chevron dropping the tray panel (`mode` = "collapsed"), or every
/// hosted icon right in the bar ("expanded"), clicks going to their apps. `pinned`
/// programs' icons stay in the bar before the chevron (which then says how many are
/// behind it); `activity_dot` marks it when one of those changed since you last
/// looked. Style "slide": no panel, the icons slide out of the chevron along the bar.
pub struct Tray {
    opts: Opts,
    pins: Vec<String>,
    icons: Vec<TrayIcon>,
    bitmaps: HashMap<String, (String, AppIcon)>,
    style: Option<Style>,
    hovered: Option<TrayHit>,
    h: f32,
    /// The chevron's width (with its count), as last measured.
    chev_w: f32,
    /// What the icons behind the chevron looked like when last seen, by id (none
    /// before the first data), and whether one changed since.
    seen: Option<HashMap<String, u64>>,
    fresh: bool,
    /// Style "slide": how far out (from, to, since), its last frame drawn, the
    /// cursor gone at, the next frame.
    slide: (f32, f32, Instant),
    settled: bool,
    left: Option<Instant>,
    beat: Instant,
}

impl Tray {
    pub fn new(opts: Opts) -> Tray {
        let now = Instant::now();
        let pins = tray_pins(&opts);
        Tray {
            opts,
            pins,
            icons: Vec::new(),
            bitmaps: HashMap::new(),
            style: None,
            hovered: None,
            h: 30.0,
            chev_w: 14.0,
            seen: None,
            fresh: false,
            slide: (0.0, 0.0, now),
            settled: true,
            left: None,
            beat: now,
        }
    }
    fn slides(&self) -> bool {
        self.opts.str("style") == Some("slide")
    }
    fn inline(&self) -> bool {
        self.opts.str("mode") == Some("expanded") && !self.slides()
    }
    fn pinned(&self, i: &TrayIcon) -> bool {
        tray_pinned(&self.pins, &i.exe)
    }
    /// The icons behind the chevron.
    fn behind(&self) -> Vec<usize> {
        (0..self.icons.len()).filter(|&i| !self.pinned(&self.icons[i])).collect()
    }
    /// The chevron says how many are behind it (with icons pinned before it).
    fn counted(&self) -> bool {
        !self.inline() && self.icons.iter().any(|i| self.pinned(i))
    }
    /// Inline cells (.tray-inline): 0.8 h square, 0.06 h apart.
    fn cell(&self) -> (f32, f32) {
        (self.h * 0.8, self.h * 0.06)
    }
    fn out(&self, now: Instant) -> f32 {
        let (from, to, since) = self.slide;
        let k = (now.saturating_duration_since(since).as_secs_f32() / TRAY_SLIDE.as_secs_f32()).min(1.0);
        from + (to - from) * (1.0 - (1.0 - k).powi(3))
    }
    fn open(&self) -> bool {
        self.slide.1 > 0.5
    }
    fn slide_to(&mut self, open: bool) {
        let now = Instant::now();
        self.slide = (self.out(now), if open { 1.0 } else { 0.0 }, now);
        self.settled = false;
        self.beat = now;
        self.left = None;
    }
    /// The row's cells now, and its width.
    fn cells(&self, now: Instant) -> (Vec<TrayCell>, f32) {
        let (side, gap) = self.cell();
        let mut out = Vec::new();
        let mut x = 0.0;
        if self.inline() {
            for i in 0..self.icons.len() {
                out.push(TrayCell { hit: TrayHit::Icon(i), x, w: side, alpha: 1.0 });
                x += side + gap;
            }
            return (out, (x - gap).max(0.0));
        }
        for (i, icon) in self.icons.iter().enumerate() {
            if self.pinned(icon) {
                out.push(TrayCell { hit: TrayHit::Icon(i), x, w: side, alpha: 1.0 });
                x += side + gap;
            }
        }
        out.push(TrayCell { hit: TrayHit::Chevron, x, w: self.chev_w, alpha: 1.0 });
        x += self.chev_w;
        if self.slides() {
            let e = self.out(now);
            let behind = self.behind();
            if e > 0.001 {
                for (k, &i) in behind.iter().enumerate() {
                    out.push(TrayCell { hit: TrayHit::Icon(i), x: x + (gap + k as f32 * (side + gap)) * e, w: side, alpha: e });
                }
            }
            x += behind.len() as f32 * (side + gap) * e;
        }
        (out, x)
    }
    fn at(&self, x: f32) -> Option<TrayHit> {
        let (cells, _) = self.cells(Instant::now());
        cells.iter().rev().find(|c| c.alpha > 0.5 && x >= c.x && x < c.x + c.w).map(|c| c.hit)
    }
    /// Each icon behind the chevron's look, by id (a hash of its picture and hint).
    fn looks(&self) -> HashMap<String, u64> {
        use std::hash::{Hash, Hasher};
        self.behind()
            .into_iter()
            .map(|i| {
                let mut h = std::collections::hash_map::DefaultHasher::new();
                (&self.icons[i].rgba, &self.icons[i].tip).hash(&mut h);
                (self.icons[i].id.clone(), h.finish())
            })
            .collect()
    }
    /// The icons behind the chevron were looked at: no dot until one changes.
    fn seen_now(&mut self) {
        self.seen = Some(self.looks());
        self.fresh = false;
    }
    fn glyph(&self) -> &'static str {
        match (self.slides(), self.open()) {
            (true, false) => "\u{E76C}",
            (true, true) => "\u{E76B}",
            _ => "\u{E70D}",
        }
    }
}

impl Widget for Tray {
    fn keys(&self) -> &'static [&'static str] {
        &["tray"]
    }
    fn tooltip(&self) -> Option<String> {
        match self.hovered {
            Some(TrayHit::Icon(i)) => self.icons.get(i).map(|i| if i.tip.is_empty() { i.name.clone() } else { i.tip.clone() }).filter(|t| !t.is_empty()),
            _ => {
                let n = if self.inline() { self.icons.len() } else { self.behind().len() };
                Some(if n == 0 { "No tray icons".into() } else { format!("{n} tray icon{}", if n == 1 { "" } else { "s" }) })
            }
        }
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
                Some(TrayIcon { id, tip, name: s("name"), exe: s("exe"), px: if fresh { decode(i) } else { None }, rgba })
            })
            .collect();
        match &self.seen {
            None => self.seen_now(),
            Some(seen) if self.opts.flag("activity_dot") == Some(true) && !self.fresh => {
                self.fresh = self.looks().iter().any(|(id, h)| seen.get(id) != Some(h));
            }
            _ => {}
        }
        true
    }
    fn width(&mut self, cx: &Ctx) -> f32 {
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
        // A bare chevron as before; with a count or sliding, a button around it.
        let g = cx.gfx.measure(self.glyph(), &crate::native::gfx::Font::icons(14.0));
        self.chev_w = if self.counted() {
            8.0 + g + 5.0 + cx.gfx.measure(&self.behind().len().to_string(), &cx.secondary(11.0)) + 8.0
        } else if self.slides() {
            self.cell().0
        } else {
            g
        };
        self.cells(Instant::now()).1
    }
    fn draw(&self, cx: &Ctx, x: f32, cy: f32, alpha: f32) {
        let g = cx.gfx;
        let (side, _) = self.cell();
        let img = self.h * 0.55;
        let (cells, _) = self.cells(Instant::now());
        for c in &cells {
            let a = alpha * c.alpha;
            let boxed = matches!(c.hit, TrayHit::Icon(_)) || self.counted() || self.slides();
            if self.hovered == Some(c.hit) && boxed {
                g.fill_round(rect(x + c.x, cy - side / 2.0, c.w, side), 8.0, with_alpha(cx.hover_bg(0.12), a));
            }
            match c.hit {
                TrayHit::Icon(i) => {
                    if let Some((_, b)) = self.icons.get(i).and_then(|icon| self.bitmaps.get(&icon.id)) {
                        b.draw(g, g.snap(x + c.x + (side - img) / 2.0), g.snap(cy - img / 2.0), img, a);
                    }
                }
                TrayHit::Chevron => {
                    let f = crate::native::gfx::Font::icons(14.0);
                    let gw = g.measure(self.glyph(), &f);
                    let gx = if self.counted() { x + c.x + 8.0 } else { x + c.x + (c.w - gw) / 2.0 };
                    g.text_at(self.glyph(), &f, gx, cy + 1.0, with_alpha(cx.look.fg, a));
                    if self.counted() {
                        g.text_at(&self.behind().len().to_string(), &cx.secondary(11.0), gx + gw + 5.0, cy, with_alpha(cx.look.dimmed(), a));
                    }
                    if self.fresh {
                        let (dx, dy) = (x + c.x + c.w - 3.0, cy - side / 2.0 + 3.0);
                        g.fill_round(rect(dx - 3.0, dy - 3.0, 6.0, 6.0), 3.0, with_alpha(cx.look.accent, a));
                    }
                }
            }
        }
    }
    fn hover(&mut self, at: Option<(f32, f32)>) -> bool {
        let h = at.and_then(|(x, _)| self.at(x));
        match at {
            None if self.open() && self.left.is_none() => self.left = Some(Instant::now()),
            Some(_) => self.left = None,
            _ => {}
        }
        let changed = h != self.hovered;
        self.hovered = h;
        changed
    }
    fn wake(&self) -> Option<Instant> {
        let slide = (!self.settled).then_some(self.beat);
        let linger = self.left.filter(|_| self.open()).map(|t| t + TRAY_LINGER);
        [slide, linger].into_iter().flatten().min()
    }
    fn tick(&mut self, now: Instant) -> bool {
        if self.left.is_some_and(|t| self.open() && now >= t + TRAY_LINGER) {
            self.slide_to(false);
        }
        if self.settled {
            return false;
        }
        if now < self.slide.2 + TRAY_SLIDE {
            self.beat = now + Duration::from_millis(15);
        } else {
            self.settled = true;
        }
        true
    }
    fn click(&mut self, button: Button, at: (f32, f32)) -> Reply {
        let Some(hit) = self.at(at.0) else { return Reply::None };
        let i = match hit {
            TrayHit::Chevron if button == Button::Left => {
                self.seen_now();
                if self.slides() {
                    let open = !self.open();
                    self.slide_to(open);
                    return Reply::Redraw;
                }
                return Reply::TogglePanel;
            }
            TrayHit::Chevron => return Reply::None,
            TrayHit::Icon(i) => i,
        };
        let (cells, _) = self.cells(Instant::now());
        let Some(c) = cells.iter().find(|c| c.hit == hit) else { return Reply::None };
        let centre = (c.x + c.w / 2.0, 0.0);
        let button = match button {
            Button::Left => "left",
            Button::Right => "right",
            Button::Middle => return Reply::None,
        };
        Reply::Do(Act::Tray { id: self.icons[i].id.clone(), button, at: centre })
    }
}
