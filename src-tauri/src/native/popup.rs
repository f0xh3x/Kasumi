//! Pop-ups out of a notch (src/main.ts showOsd / showNotification /
//! showKeyboard): a level (volume, brightness) or a word (keyboard layout, Caps
//! Lock, "update") as a pill the notch grows into; a new notification in the
//! same pill; the keyboard peek, a small notch at the language widget.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde_json::Value;

use super::gfx::{rect, rgba, white, with_alpha, Font, Gfx};
use super::widgets::icon::{decode, AppIcon, Pixels, Style};
use super::widgets::{fit_text, Act, Look};
use super::Spring;
use crate::engine::Layout;

/// What the pill shows.
pub enum Show {
    /// A level (0..1) with its glyph; or a word instead of the level bar.
    Level { glyph: &'static str, level: f32, muted: bool, update: bool, text: Option<String>, icon: Option<String> },
    /// A notification: (style, app, aumid, title, body), and when it came (the icon's bounce).
    Note { style: String, app: String, aumid: String, title: String, body: String, at: Instant },
}

pub struct Popups {
    pub osd: Spring,
    pub until: Option<Instant>,
    /// The notch it grows out of (a notch id or "spot:<side>"; None: the centre one).
    pub notch: Option<String>,
    /// Its size while it's a word or a notification (None: [osd] width × height).
    pub size: Option<(f64, f64)>,
    pub show: Option<Show>,
    pub peek: Spring,
    pub peek_until: Option<Instant>,
    pub peek_text: String,
    /// The peek's centre x (the language widget's).
    pub peek_x: f64,
    icons: HashMap<String, AppIcon>,
    pixels: HashMap<String, Pixels>,
    asked: Vec<String>,
    pub pending: Option<Act>,
}

fn volume_glyph(pct: f32, muted: bool) -> &'static str {
    if muted {
        "\u{E74F}"
    } else if pct == 0.0 {
        "\u{E992}"
    } else if pct < 34.0 {
        "\u{E993}"
    } else if pct < 67.0 {
        "\u{E994}"
    } else {
        "\u{E995}"
    }
}

impl Popups {
    pub fn new(a: &crate::config::Animation) -> Popups {
        let mut osd = Spring::new(0.0, a.expand_stiffness, a.expand_damping);
        osd.configure(a.expand_stiffness, a.expand_damping, a);
        let peek = osd.clone();
        Popups { osd, until: None, notch: None, size: None, show: None, peek, peek_until: None, peek_text: String::new(), peek_x: 0.0, icons: HashMap::new(), pixels: HashMap::new(), asked: Vec::new(), pending: None }
    }

    pub fn configure(&mut self, a: &crate::config::Animation) {
        self.osd.configure(a.expand_stiffness, a.expand_damping, a);
        self.peek.configure(a.expand_stiffness, a.expand_damping, a);
    }

    /// A level or word (`notch://osd`).
    pub fn osd(&mut self, g: &Gfx, look: &Look, l: &Layout, e: &Value) {
        self.notch = e.get("notch").and_then(|n| n.as_str()).map(String::from);
        let kind = e.get("kind").and_then(|k| k.as_str()).unwrap_or("volume");
        let level = e.get("level").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
        let muted = e.get("muted").and_then(|v| v.as_bool()).unwrap_or(false);
        let pct = (level * 100.0).round();
        let glyph = match kind {
            "keyboard" | "caps" => "\u{E765}",
            "update" => "\u{E896}",
            "progress" => "\u{E8C8}",
            "volume" => volume_glyph(pct, muted),
            _ => "\u{E706}",
        };
        let text = e.get("text").and_then(|t| t.as_str()).map(String::from);
        // A word: a pill around it, not the level pill's width.
        self.size = text.as_ref().map(|t| {
            let w = 18.0 + 10.0 + g.measure(t, &Font::new(look.main.clone(), 12.0)) + 8.0;
            ((w as f64 + 2.0 * l.geometry.notch_corner_radius + 8.0).max(100.0).round(), l.osd.height)
        });
        // An app's own level (scrolling over it in the dock): its icon for the glyph.
        let icon = e.get("app").and_then(|a| a.as_str()).map(String::from);
        self.show = Some(Show::Level { glyph, level, muted, update: kind == "update", text, icon });
        self.osd.target = 1.0;
        let ms = e.get("ms").and_then(|m| m.as_u64()).unwrap_or(l.osd.duration_ms);
        self.until = Some(Instant::now() + Duration::from_millis(ms));
    }

    /// A new notification out of the notch (`notch://notification`).
    pub fn notification(&mut self, g: &Gfx, look: &Look, l: &Layout, n: &Value) {
        self.notch = n.get("notch").and_then(|v| v.as_str()).map(String::from);
        let s = |k: &str| n.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
        let style = match l.osd.notification_style.as_str() {
            "two" | "compact" => l.osd.notification_style.clone(),
            _ => "line".to_string(),
        };
        let (app, aumid, title, body) = (s("app"), s("aumid"), s("title"), s("body"));
        let widest = l.osd.notification_width.max(160.0).max(if l.osd.notification_width > 0.0 { 0.0 } else { 420.0 });
        let main = Font::new(look.main.clone(), look.font_size);
        let small = Font::new(look.secondary.clone(), 12.0);
        let content = 18.0
            + 8.0
            + match style.as_str() {
                "compact" => g.measure(&app, &main),
                "two" => g.measure(&[app.as_str(), title.as_str()].iter().filter(|t| !t.is_empty()).cloned().collect::<Vec<_>>().join(" · "), &Font::new(look.main.clone(), 12.0)).max(g.measure(if body.is_empty() { &title } else { &body }, &small)),
                _ => g.measure(&app, &main) + 8.0 + g.measure(&[title.as_str(), body.as_str()].iter().filter(|t| !t.is_empty()).cloned().collect::<Vec<_>>().join(" — "), &small),
            }
            + 8.0;
        // (2 px of slack: a width rounded down would cut the text by its last letter.)
        let w = if l.osd.notification_fit { (content as f64 + 2.0 + 2.0 * l.geometry.notch_corner_radius + 8.0).clamp(160.0, widest).ceil() } else { widest };
        self.size = Some((w, if style == "two" { l.osd.height + 20.0 } else { l.osd.height }));
        if !aumid.is_empty() && !self.asked.contains(&aumid) {
            self.asked.push(aumid.clone());
            let id = aumid.clone();
            self.pending = Some(Act::call(move |_| {
                if let Ok(icon) = tauri::async_runtime::block_on(crate::launcher::launcher_icon(id.clone())) {
                    let mut m = serde_json::Map::new();
                    m.insert(id, icon);
                    crate::native::post(crate::native::Msg::Entries("appIcons".into(), m));
                }
            }));
        }
        self.show = Some(Show::Note { style, app, aumid, title, body, at: Instant::now() });
        self.osd.target = 1.0;
        let ms = if l.osd.notification_ms > 0 { l.osd.notification_ms } else { 5000 };
        self.until = Some(Instant::now() + Duration::from_millis(ms));
    }

    /// The keyboard peek with the layout's code, at the language widget (x: its centre).
    pub fn peek(&mut self, text: String, x: f64, ms: u64) {
        self.peek_text = text;
        self.peek_x = x;
        self.peek.target = 1.0;
        self.peek_until = Some(Instant::now() + Duration::from_millis(ms));
    }

    pub fn data(&mut self, v: &Value) -> bool {
        let Some(map) = v.as_object() else { return false };
        for (id, icon) in map {
            if !self.icons.contains_key(id) {
                if let Some(px) = decode(icon) {
                    self.pixels.insert(id.clone(), px);
                }
            }
        }
        true
    }

    pub fn wake(&self) -> Option<Instant> {
        [self.until, self.peek_until].into_iter().flatten().min()
    }

    /// Retracts what has been out long enough; true if something changed.
    pub fn tick(&mut self, now: Instant) -> bool {
        let mut changed = false;
        if self.until.is_some_and(|t| now >= t) {
            self.until = None;
            self.osd.target = 0.0;
            changed = true;
        }
        if self.peek_until.is_some_and(|t| now >= t) {
            self.peek_until = None;
            self.peek.target = 0.0;
            changed = true;
        }
        changed
    }

    /// The notification icon's bounce (styles.css note-bounce: 650 ms after 150 ms).
    pub fn bouncing(&self, now: Instant) -> bool {
        matches!(&self.show, Some(Show::Note { at, .. }) if now.saturating_duration_since(*at) < Duration::from_millis(820))
    }

    /// Draws the pill's content, `inner` wide, centred on (cx, cy) (or along the
    /// side it grows from: -1 left, 1 right).
    #[allow(clippy::too_many_arguments)]
    pub fn draw(&mut self, g: &Gfx, look: &Look, accent: bool, cx: f32, cy: f32, inner: f32, side: i32, alpha: f32, nudge: bool) {
        for (id, px) in self.pixels.drain() {
            if let Some(i) = AppIcon::new(g, &px, Style::Color) {
                self.icons.insert(id, i);
            }
        }
        let Some(show) = &self.show else { return };
        let fg = if matches!(show, Show::Level { update: true, .. }) { look.accent } else { look.fg };
        let left = cx - inner / 2.0;
        match show {
            Show::Level { glyph, level, muted, text, icon, .. } => {
                let app_icon = icon.as_ref().and_then(|k| self.icons.get(k));
                let f = Font::icons(15.0);
                match text {
                    // A word: icon and text centred.
                    Some(t) => {
                        let tf = Font::new(look.main.clone(), 12.0);
                        let w = 18.0 + 10.0 + g.measure(t, &tf);
                        let x0 = cx - w / 2.0;
                        let gw = g.measure(glyph, &f);
                        g.text_at(glyph, &f, x0 + (18.0 - gw) / 2.0, cy, with_alpha(fg, alpha));
                        g.text_at(t, &tf, x0 + 28.0, cy, with_alpha(fg, alpha));
                    }
                    None => {
                        let x0 = left + 4.0;
                        match app_icon {
                            Some(ic) => ic.draw(g, x0, cy - 9.0, 18.0, alpha),
                            None => {
                                let gw = g.measure(glyph, &f);
                                g.text_at(glyph, &f, x0 + (18.0 - gw) / 2.0, cy, with_alpha(fg, alpha));
                            }
                        }
                        let value = format!("{}", (level * 100.0).round());
                        let vf = Font::new(look.main.clone(), 12.0);
                        let vw = g.measure(&value, &vf);
                        let track = inner - 8.0 - 18.0 - 10.0 - 10.0 - 26.0;
                        let tx = x0 + 28.0;
                        g.fill_round(rect(tx, cy - 2.0, track, 4.0), 2.0, white(0.2 * alpha));
                        let fill = if *muted { white(0.35) } else if accent { look.accent } else { fg };
                        g.fill_round(rect(tx, cy - 2.0, track * level.clamp(0.0, 1.0), 4.0), 2.0, with_alpha(fill, alpha));
                        g.text_at(&value, &vf, left + inner - 4.0 - vw, cy, with_alpha(fg, alpha));
                    }
                }
            }
            Show::Note { style, app, aumid, title, body, at } => {
                let main = Font::new(look.main.clone(), look.font_size);
                let small = Font::new(look.secondary.clone(), 12.0);
                // The parts, then laid out centred (or along the side it grows from).
                let avail = inner - 8.0 - 18.0 - 8.0;
                let (head, text) = match style.as_str() {
                    "compact" => (fit_text(g, app, &main, avail), String::new()),
                    "two" => (fit_text(g, &[app.as_str(), title.as_str()].iter().filter(|t| !t.is_empty()).cloned().collect::<Vec<_>>().join(" · "), &Font::new(look.main.clone(), 12.0), avail), fit_text(g, if body.is_empty() { title } else { body }, &small, avail)),
                    _ => {
                        let aw = g.measure(app, &main).min(avail);
                        (fit_text(g, app, &main, aw), fit_text(g, &[title.as_str(), body.as_str()].iter().filter(|t| !t.is_empty()).cloned().collect::<Vec<_>>().join(" — "), &small, (avail - aw - 8.0).max(0.0)))
                    }
                };
                let two = style == "two";
                let head_font = if two { Font::new(look.main.clone(), 12.0) } else { main.clone() };
                let hw = g.measure(&head, &head_font);
                let tw = g.measure(&text, &small);
                let width = 18.0 + 8.0 + if two { hw.max(tw) } else { hw + if text.is_empty() { 0.0 } else { 8.0 + tw } };
                let x0 = match side {
                    -1 => left + 4.0,
                    1 => left + inner - 4.0 - width,
                    _ => cx - width / 2.0,
                };
                // The icon bounces once as it comes (notification nudge).
                let t = Instant::now().saturating_duration_since(*at).as_secs_f32() * 1000.0 - 150.0;
                let bounce = if nudge && (0.0..650.0).contains(&t) {
                    let k = t / 650.0;
                    let keys = [(0.0, 0.0), (0.3, -5.0), (0.55, 0.0), (0.75, -2.0), (1.0, 0.0)];
                    let i = keys.iter().rposition(|p| p.0 <= k).unwrap_or(0).min(keys.len() - 2);
                    let (a, b) = (keys[i], keys[i + 1]);
                    a.1 + (b.1 - a.1) * ((k - a.0) / (b.0 - a.0))
                } else {
                    0.0
                };
                match self.icons.get(aumid) {
                    Some(icon) => icon.draw(g, x0, cy - 9.0 + bounce, 18.0, alpha),
                    None => {
                        let f = Font::icons(15.0);
                        let gw = g.measure("\u{EA8F}", &f);
                        g.text_at("\u{EA8F}", &f, x0 + (18.0 - gw) / 2.0, cy + bounce, with_alpha(look.fg, alpha));
                    }
                }
                let tx = x0 + 26.0;
                if two {
                    let (hh, th) = (g.text("Ag", &head_font).map_or(15.0, |t| t.height), g.text("Ag", &small).map_or(15.0, |t| t.height));
                    let top = cy - (hh + 1.0 + th) / 2.0;
                    g.text_at(&head, &head_font, tx, top + hh / 2.0, with_alpha(look.fg, alpha));
                    g.text_at(&text, &small, tx, top + hh + 1.0 + th / 2.0, white(0.75 * alpha));
                } else {
                    g.text_at(&head, &head_font, tx, cy, with_alpha(look.fg, alpha));
                    if !text.is_empty() {
                        g.text_at(&text, &small, tx + hw + 8.0, cy, white(0.75 * alpha));
                    }
                }
                let _ = rgba;
            }
        }
    }

    /// The keyboard peek's content (keyboard glyph and the code), centred on (cx, cy).
    pub fn draw_peek(&self, g: &Gfx, look: &Look, cx: f32, cy: f32, alpha: f32) {
        let f = Font::icons(14.0);
        let tf = Font::new(look.main.clone(), look.font_size);
        let w = g.measure("\u{E765}", &f) + 6.0 + g.measure(&self.peek_text, &tf);
        let x0 = cx - w / 2.0;
        let gw = g.measure("\u{E765}", &f);
        g.text_at("\u{E765}", &f, x0, cy, with_alpha(look.fg, alpha));
        g.text_at(&self.peek_text, &tf, x0 + gw + 6.0, cy, with_alpha(look.fg, alpha));
    }
}
