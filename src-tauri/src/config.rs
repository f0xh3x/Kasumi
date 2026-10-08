use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, fs, path::PathBuf, sync::mpsc::Sender};

pub const DEFAULT_CONFIG: &str = include_str!("../default-config.toml");

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub timing: Timing,
    #[serde(default)]
    pub geometry: Geometry,
    #[serde(default)]
    pub animation: Animation,
    #[serde(default)]
    pub bar: Bar,
    #[serde(default)]
    pub notches: Vec<Notch>,
    /// Which side each widget sits on and which get their own notch. When
    /// absent, the legacy `[bar]` lists + `[[notches]]` tables are used.
    #[serde(default)]
    pub layout: Option<LayoutCfg>,
    /// How notches on each side behave ([layout] model).
    #[serde(default)]
    pub notch: NotchCfg,
    #[serde(default)]
    pub monitors: Vec<MonitorRule>,
    #[serde(default)]
    pub general: General,
    #[serde(default)]
    pub theme: Theme,
    #[serde(default)]
    pub osd: Osd,
    #[serde(default)]
    pub widgets: toml::Table,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Timing {
    pub poll_ms: u64,
    pub dwell_ms: u64,
    pub grace_ms: u64,
    pub edge_px: f64,
    /// Size of the top-left hot corner that opens the bar (logical px).
    pub hot_corner: f64,
    pub hot_corner_enabled: bool,
    /// Legacy single position ("top_left" / "top_right" / "top_edge"); the
    /// switches below take over once set.
    pub hot_corner_position: String,
    pub hot_top_left: Option<bool>,
    pub hot_top_right: Option<bool>,
    /// Anywhere along the top edge opens the bar.
    pub hot_top_edge: Option<bool>,
}

impl Default for Timing {
    fn default() -> Self {
        Self { poll_ms: 16, dwell_ms: 120, grace_ms: 300, edge_px: 2.0, hot_corner: 12.0, hot_corner_enabled: true, hot_corner_position: "top_left".into(), hot_top_left: None, hot_top_right: None, hot_top_edge: None }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, rename_all(serialize = "camelCase"))]
pub struct Geometry {
    pub expanded_height: f64,
    pub corner_radius: f64,
    pub concave_radius: f64,
    pub notch_height: f64,
    pub notch_corner_radius: f64,
    pub panel_concave_radius: f64,
    pub panel_corner_radius: f64,
}

impl Default for Geometry {
    fn default() -> Self {
        Self {
            expanded_height: 40.0,
            corner_radius: 14.0,
            concave_radius: 10.0,
            notch_height: 30.0,
            notch_corner_radius: 12.0,
            panel_concave_radius: 10.0,
            panel_corner_radius: 12.0,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, rename_all(serialize = "camelCase"))]
pub struct Animation {
    pub expand_stiffness: f64,
    pub expand_damping: f64,
    pub spread_stiffness: f64,
    pub spread_damping: f64,
    pub panel_stiffness: f64,
    pub panel_damping: f64,
    pub mass: f64,
    /// "drop": the full-width bar slides down out of the top edge.
    /// "spread": the bar grows out of the notches sideways.
    pub style: String,
}

impl Default for Animation {
    fn default() -> Self {
        Self {
            expand_stiffness: 420.0,
            expand_damping: 32.0,
            spread_stiffness: 260.0,
            spread_damping: 28.0,
            panel_stiffness: 380.0,
            panel_damping: 30.0,
            mass: 1.0,
            style: "drop".into(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct Bar {
    pub left: Vec<String>,
    pub center: Vec<String>,
    pub right: Vec<String>,
    pub padding: f64,
    pub gap: f64,
}

impl Default for Bar {
    fn default() -> Self {
        Self { left: vec![], center: vec![], right: vec![], padding: 18.0, gap: 18.0 }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct General {
    /// Hide the overlay while a fullscreen app (game, video) is in front.
    pub hide_in_fullscreen: bool,
}

impl Default for General {
    fn default() -> Self {
        Self { hide_in_fullscreen: true }
    }
}

/// Colors and text size of the notch, bar and panels.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, rename_all(serialize = "camelCase"))]
pub struct Theme {
    pub background: String,
    pub text: String,
    /// Opacity of secondary text (0..1), derived from `text`.
    pub dim_opacity: f64,
    pub accent: String,
    /// Hot readings (temps above their warning level).
    pub warn: String,
    pub font_size: f64,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            background: "#000000".into(),
            text: "#ffffff".into(),
            dim_opacity: 0.45,
            accent: "#ffffff".into(),
            warn: "#ff9f43".into(),
            font_size: 13.0,
        }
    }
}

/// On-screen display: a notch briefly grows into a pill showing a level.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, rename_all(serialize = "camelCase"))]
pub struct Osd {
    pub enabled: bool,
    /// Notch that grows into the pill (falls back to the monitor's first notch).
    pub notch: String,
    pub width: f64,
    pub height: f64,
    pub duration_ms: u64,
    /// Volume change per key press, in percent.
    pub volume_step: f64,
    /// Take over the volume keys (suppresses the Windows volume flyout).
    pub replace_volume_keys: bool,
    /// Show brightness changes of the built-in panel (Fn keys).
    pub brightness: bool,
    /// Pop a small notch at the language widget when the keyboard layout changes.
    pub keyboard: bool,
    /// Width of that keyboard-layout peek notch.
    pub keyboard_width: f64,
}

impl Default for Osd {
    fn default() -> Self {
        Self {
            enabled: true,
            notch: "clock".into(),
            width: 230.0,
            height: 34.0,
            duration_ms: 1500,
            volume_step: 2.0,
            replace_volume_keys: true,
            brightness: true,
            keyboard: true,
            keyboard_width: 120.0,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Default, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Anchor {
    Left,
    #[default]
    Center,
    Right,
}

/// A horizontal distance: a number of px, or a string like "12%" / "40px".
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Length {
    Px(f64),
    Text(String),
}

impl Default for Length {
    fn default() -> Self {
        Length::Px(0.0)
    }
}

impl Length {
    pub fn resolve(&self, total: f64) -> f64 {
        match self {
            Length::Px(v) => *v,
            Length::Text(s) => {
                let s = s.trim();
                if let Some(pct) = s.strip_suffix('%') {
                    pct.trim().parse::<f64>().unwrap_or(0.0) / 100.0 * total
                } else {
                    s.trim_end_matches("px").trim().parse().unwrap_or(0.0)
                }
            }
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Notch {
    pub id: String,
    #[serde(default)]
    pub anchor: Anchor,
    #[serde(default)]
    pub offset: Length,
    #[serde(default = "default_notch_width")]
    pub width: f64,
    pub height: Option<f64>,
    /// Hover zone width: px, or "%" of the notch's visible width at the top edge.
    pub trigger_width: Option<Length>,
    #[serde(default)]
    pub widgets: Vec<String>,
    pub panel: Option<String>,
    /// Left/right notches only: hug the screen corner (square edge) while no
    /// app window reaches under it; otherwise sit at `offset`.
    #[serde(default)]
    pub dock: bool,
    /// Docked notches slide to `offset` while an app window reaches under them.
    #[serde(default = "default_true")]
    pub avoid_windows: bool,
    #[serde(default = "default_panel_width")]
    pub panel_width: f64,
    #[serde(default = "default_panel_height")]
    pub panel_height: f64,
    /// Width follows the content (measured by the overlay); [layout] model only.
    #[serde(skip)]
    pub auto_width: bool,
}

fn default_notch_width() -> f64 {
    96.0
}
fn default_panel_width() -> f64 {
    260.0
}
fn default_panel_height() -> f64 {
    236.0
}

#[derive(Debug, Clone, Deserialize)]
pub struct MonitorRule {
    #[serde(rename = "match", default = "default_match")]
    pub matcher: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Notch ids shown on this monitor; all notches when omitted. (Legacy model.)
    pub notches: Option<Vec<String>>,
    /// Legacy model: this monitor's own bar lists.
    pub bar: Option<Bar>,
    /// This monitor's own layout; absent = follow the global `[layout]`.
    pub layout: Option<LayoutCfg>,
}

/// Widget placement: a side for every shown widget, and which of them show as
/// their own notch (on that side).
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct LayoutCfg {
    pub left: Vec<String>,
    pub center: Vec<String>,
    pub right: Vec<String>,
    pub notch: Vec<String>,
}

/// `[notch.left]` / `[notch.center]` / `[notch.right]`.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct NotchSide {
    /// Left/right: hug the screen corner.
    pub hug: bool,
    /// While hugging: slide to `offset` when an app window's caption buttons are there.
    pub avoid_windows: bool,
    /// From the side's edge (center: shift from the middle) when not hugging.
    pub offset: Length,
}

impl Default for NotchSide {
    fn default() -> Self {
        Self { hug: true, avoid_windows: true, offset: Length::Px(150.0) }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct NotchCfg {
    /// Notch widgets on the same side share one notch.
    pub merge: bool,
    /// Space between separate notches on a side (beyond their flares).
    pub gap: f64,
    /// Notches that would touch or overlap are drawn as one (widgets stay put).
    pub combine_overlapping: bool,
    pub left: NotchSide,
    pub center: NotchSide,
    pub right: NotchSide,
}

impl Default for NotchCfg {
    fn default() -> Self {
        Self {
            merge: true,
            gap: 8.0,
            combine_overlapping: true,
            left: NotchSide::default(),
            center: NotchSide { hug: false, avoid_windows: false, offset: Length::Px(0.0) },
            right: NotchSide::default(),
        }
    }
}

fn default_match() -> String {
    "*".into()
}

fn default_true() -> bool {
    true
}

impl Config {
    /// A widget is on unless its `[widgets.<name>]` table says `enabled = false`.
    pub fn widget_enabled(&self, name: &str) -> bool {
        self.widgets.get(name).and_then(|t| t.get("enabled")).and_then(|v| v.as_bool()) != Some(false)
    }

    /// `widgets.<name>.hide_when_maximized`: the widget leaves its notch while a
    /// window covers the notch's spot, instead of moving aside with it.
    pub fn widget_hides_when_maximized(&self, name: &str) -> bool {
        self.widgets.get(name).and_then(|t| t.get("hide_when_maximized")).and_then(|v| v.as_bool()) == Some(true)
    }

    /// The notches and bar groups a monitor shows, from the new `[layout]` model
    /// (with the monitor rule's own layout, if any) or the legacy model.
    pub fn resolve_layout(&self, rule: Option<&MonitorRule>) -> (Vec<Notch>, Bar) {
        let Some(global) = &self.layout else {
            let wanted = rule.and_then(|r| r.notches.as_ref());
            let notches = self.notches.iter().filter(|n| wanted.is_none_or(|ids| ids.contains(&n.id))).cloned().collect();
            let bar = rule.and_then(|r| r.bar.clone()).unwrap_or_else(|| self.bar.clone());
            return (notches, bar);
        };
        let layout = rule.and_then(|r| r.layout.as_ref()).unwrap_or(global);
        let mut notches = Vec::new();
        let sides = [
            ("left", Anchor::Left, &layout.left, &self.notch.left),
            ("center", Anchor::Center, &layout.center, &self.notch.center),
            ("right", Anchor::Right, &layout.right, &self.notch.right),
        ];
        for (name, anchor, list, side) in sides {
            let ws: Vec<String> =
                list.iter().filter(|w| layout.notch.contains(w) && self.widget_enabled(w)).cloned().collect();
            if ws.is_empty() {
                continue;
            }
            let make = |id: String, widgets: Vec<String>| Notch {
                id,
                anchor,
                offset: side.offset.clone(),
                // A first guess; the overlay measures the real content width.
                width: 60.0 + 70.0 * widgets.len() as f64,
                height: None,
                trigger_width: None,
                widgets,
                panel: None,
                dock: anchor != Anchor::Center && side.hug,
                avoid_windows: side.avoid_windows,
                panel_width: 0.0,
                panel_height: 0.0,
                auto_width: true,
            };
            if self.notch.merge {
                notches.push(make(name.to_string(), ws));
            } else {
                notches.extend(ws.into_iter().map(|w| make(w.clone(), vec![w])));
            }
        }
        // Every widget is in the open bar, notch widgets included (their notch
        // content fades out as the bar's copy fades in).
        let in_bar = |list: &Vec<String>| -> Vec<String> { list.iter().filter(|w| self.widget_enabled(w)).cloned().collect() };
        let bar = Bar {
            left: in_bar(&layout.left),
            center: in_bar(&layout.center),
            right: in_bar(&layout.right),
            padding: self.bar.padding,
            gap: self.bar.gap,
        };
        (notches, bar)
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        let mut seen = HashSet::new();
        for n in &self.notches {
            if !seen.insert(n.id.as_str()) {
                return Err(format!("duplicate notch id '{}'", n.id));
            }
        }
        if self.geometry.expanded_height <= 0.0 {
            return Err("geometry.expanded_height must be > 0".into());
        }
        Ok(())
    }
}

pub fn dir() -> PathBuf {
    PathBuf::from(std::env::var("APPDATA").unwrap_or_else(|_| ".".into())).join("StatusNotch")
}

pub fn path() -> PathBuf {
    dir().join("config.toml")
}

/// Loads the user config, writing the default one first if none exists.
pub fn load() -> Result<Config, String> {
    let p = path();
    if !p.exists() {
        fs::create_dir_all(dir()).map_err(|e| e.to_string())?;
        fs::write(&p, DEFAULT_CONFIG).map_err(|e| e.to_string())?;
    }
    let text = fs::read_to_string(&p).map_err(|e| e.to_string())?;
    let cfg: Config = toml::from_str(&text).map_err(|e| e.to_string())?;
    cfg.validate()?;
    Ok(cfg)
}

pub fn fallback() -> Config {
    toml::from_str(DEFAULT_CONFIG).expect("built-in default config must parse")
}

/// Sends `()` on every change to config.toml. Keep the returned watcher alive.
pub fn watch(tx: Sender<()>) -> Option<RecommendedWatcher> {
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(ev) = res {
            if ev.paths.iter().any(|p| p.file_name().is_some_and(|f| f == "config.toml")) {
                let _ = tx.send(());
            }
        }
    })
    .ok()?;
    watcher.watch(&dir(), RecursiveMode::NonRecursive).ok()?;
    Some(watcher)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_uses_the_layout_model() {
        let cfg = fallback();
        let (notches, bar) = cfg.resolve_layout(cfg.monitors.first());
        let ids: Vec<_> = notches.iter().map(|n| (n.id.as_str(), n.widgets.clone())).collect();
        assert_eq!(ids, [("center", vec!["clock".to_string()])]);
        assert!(notches[0].auto_width);
        // Shipped minimal: only the menu, clock and tray are on; the rest are opt-in.
        assert_eq!(bar.left, ["menu"]);
        assert_eq!(bar.center, ["clock"]);
        assert_eq!(bar.right, ["tray"]);
        for w in ["temps", "media", "weather", "language"] {
            assert!(!cfg.widget_enabled(w), "{w} ships disabled");
        }
    }

    #[test]
    fn side_notches_merge_hug_and_skip_disabled_widgets() {
        let cfg: Config = toml::from_str(
            r#"
            [layout]
            left = ["menu", "tray", "language"]
            center = ["clock"]
            right = ["media", "temps", "weather"]
            notch = ["menu", "language", "clock", "temps", "weather"]

            [notch.right]
            hug = false
            offset = 120

            [widgets.weather]
            enabled = false

            [[monitors]]
            match = "ARZOPA"
            layout = { left = ["menu"], right = ["temps"], notch = [] }
            "#,
        )
        .unwrap();

        let (notches, bar) = cfg.resolve_layout(None);
        let left = &notches[0];
        assert_eq!((left.id.as_str(), left.anchor), ("left", Anchor::Left));
        assert_eq!(left.widgets, ["menu", "language"]); // tray isn't a notch widget
        assert!(left.dock, "side notches hug their corner by default");
        let right = notches.iter().find(|n| n.id == "right").unwrap();
        assert_eq!(right.widgets, ["temps"]); // weather is disabled
        assert!(!right.dock);
        assert_eq!(right.offset.resolve(1000.0), 120.0);
        assert_eq!(bar.left, ["menu", "tray", "language"]);
        assert_eq!(bar.right, ["media", "temps"]);

        // A monitor's own layout replaces the global one.
        let (notches, bar) = cfg.resolve_layout(cfg.monitors.first());
        assert!(notches.is_empty());
        assert_eq!(bar.right, ["temps"]);
    }

    #[test]
    fn unmerged_notches_are_one_per_widget() {
        let cfg: Config = toml::from_str(
            r#"
            [layout]
            left = ["menu", "language"]
            notch = ["menu", "language"]
            [notch]
            merge = false
            "#,
        )
        .unwrap();
        let (notches, _) = cfg.resolve_layout(None);
        let ids: Vec<_> = notches.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids, ["menu", "language"]);
    }
}
