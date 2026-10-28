//! The weather widget (src/widgets/weather.ts): the condition's icon and the
//! temperature from Open-Meteo.

use std::time::Instant;

use serde_json::Value;

use super::{row_draw, row_width, Button, Ctx, Opts, Piece, Reply, Roll, Widget};
use crate::native::gfx::Color;
use crate::native::icons::*;

const SUNNY: &str = "\u{E706}";
const CLEAR_NIGHT: &str = "\u{EC46}";

/// A WMO weather code as an icon: clear sky in Segoe Fluent glyphs (as YASB),
/// the rest Fluent UI System Icons, drawn the same way.
pub fn icon(code: i64, day: bool, size: f32, color: Color) -> Piece {
    let svg = |d: &'static str| Piece::Svg { d, size, color };
    match code {
        0 => Piece::Glyph { s: if day { SUNNY } else { CLEAR_NIGHT }.into(), size, color, dy: 1.0 },
        1 | 2 => svg(if day { WEATHER_PARTLY_CLOUDY_DAY } else { WEATHER_PARTLY_CLOUDY_NIGHT }),
        3 => svg(WEATHER_CLOUDY),
        45 | 48 => svg(WEATHER_FOG),
        51..=57 => svg(WEATHER_DRIZZLE),
        66 | 67 => svg(WEATHER_RAIN_SNOW),
        61..=65 => svg(WEATHER_RAIN),
        80..=82 => svg(if day { WEATHER_RAIN_SHOWERS_DAY } else { WEATHER_RAIN_SHOWERS_NIGHT }),
        71..=77 => svg(WEATHER_SNOW),
        85 | 86 => svg(if day { WEATHER_SNOW_SHOWER_DAY } else { WEATHER_SNOW_SHOWER_NIGHT }),
        c if c >= 95 => svg(WEATHER_THUNDERSTORM),
        _ => svg(WEATHER_CLOUDY),
    }
}

/// A temperature as the pages show it: "7°".
pub fn deg(v: f64) -> String {
    format!("{}°", v.round())
}

pub struct Weather {
    opts: Opts,
    /// The condition (code, day) and temperature; None until there's data.
    now: Option<(i64, bool)>,
    temp: Roll,
}

impl Weather {
    pub fn new(opts: Opts) -> Weather {
        Weather { opts, now: None, temp: Roll::default() }
    }
    fn pieces(&self, cx: &Ctx) -> Vec<Piece> {
        let Some((code, day)) = self.now else { return Vec::new() };
        vec![icon(code, day, 16.0, cx.look.fg), Piece::Gap(6.0), self.temp.piece(cx.body(), cx.look.fg)]
    }
}

impl Widget for Weather {
    fn tooltip(&self) -> Option<String> {
        let (code, _) = self.now?;
        // Open-Meteo's weather codes, in words.
        let what = match code {
            0 => "Clear",
            1 | 2 => "Partly cloudy",
            3 => "Cloudy",
            45 | 48 => "Fog",
            51..=57 => "Drizzle",
            66 | 67 => "Freezing rain",
            61..=65 => "Rain",
            80..=82 => "Rain showers",
            71..=77 => "Snow",
            85 | 86 => "Snow showers",
            c if c >= 95 => "Thunderstorm",
            _ => "Cloudy",
        };
        Some(format!("{what}, {}", self.temp.text))
    }
    fn keys(&self) -> &'static [&'static str] {
        &["weather"]
    }
    fn data(&mut self, _key: &str, v: &Value) -> bool {
        if v.is_null() {
            let had = self.now.is_some();
            self.now = None;
            return had;
        }
        let now = Some((v.get("code").and_then(|c| c.as_i64()).unwrap_or(3), v.get("isDay").and_then(|d| d.as_bool()).unwrap_or(true)));
        let changed = self.temp.set(deg(v.get("temp").and_then(|t| t.as_f64()).unwrap_or(0.0)), self.opts.roll) || now != self.now;
        self.now = now;
        changed
    }
    fn width(&mut self, cx: &Ctx) -> f32 {
        row_width(cx.gfx, &self.pieces(cx))
    }
    fn draw(&self, cx: &Ctx, x: f32, cy: f32, alpha: f32) {
        row_draw(cx.gfx, &self.pieces(cx), x, cy, alpha);
    }
    fn animating(&self, now: Instant) -> bool {
        self.temp.animating(now)
    }
    fn click(&mut self, button: Button, _: (f32, f32)) -> Reply {
        if button == Button::Left { Reply::TogglePanel } else { Reply::None }
    }
}
