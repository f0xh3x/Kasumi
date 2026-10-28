use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    path::PathBuf,
    sync::mpsc::Sender,
};

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
    /// Bars and docks (`[surfaces.<id>]`). When absent, there is one: the top bar,
    /// laid out by `[layout]` (or the legacy model).
    #[serde(default)]
    pub surfaces: Option<BTreeMap<String, SurfaceCfg>>,
    /// Heights of the named surface sizes.
    #[serde(default)]
    pub sizes: Sizes,
    #[serde(default)]
    pub monitors: Vec<MonitorRule>,
    #[serde(default)]
    pub general: General,
    #[serde(default)]
    pub shell: Shell,
    #[serde(default)]
    pub launcher: LauncherCfg,
    #[serde(default)]
    pub alt_tab: AltTabCfg,
    #[serde(default)]
    pub lang_switch: LangSwitchCfg,
    #[serde(default)]
    pub context_menu: ContextMenuCfg,
    #[serde(default)]
    pub bar_menu: BarMenuCfg,
    #[serde(default)]
    pub launcher_menu: LauncherMenuCfg,
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
    /// How a panel's content shows as it opens: "slide" (moves with the panel's
    /// edge), "reveal" (stays put, the panel uncovers it), "fade" (stays put, fades in).
    pub panel_content: String,
    /// The same for the open bar's row of widgets.
    pub bar_content: String,
    /// Speed of opening and of closing (springs run this many times faster; 1 = as tuned).
    pub open_speed: f64,
    pub close_speed: f64,
    /// A notch grows a little as the cursor comes near it.
    pub hover_nudge: bool,
    /// Widget effects, each also per widget (`[widgets.<w>] anim_<key>`): a panel
    /// shows its content one piece after another (cascade); numbers roll to new
    /// values (rolling); dock icons grow under the cursor (magnify); the bell
    /// shakes for a new notification (notification_nudge).
    pub cascade: bool,
    pub rolling: bool,
    pub magnify: bool,
    /// The line under the app in front travels to the next one (tasks).
    pub marker_glide: bool,
    pub notification_nudge: bool,
    /// Springs may overshoot (as damped as set); off: never past where they head.
    pub bounce: bool,
    /// Context menus (native/ctxmenu): how one opens ("expand": grows out of where it
    /// opens, submenus sideways; "fade"; "none"), its entries one after another, its
    /// spring.
    pub menu_open: String,
    pub menu_cascade: bool,
    pub menu_stiffness: f64,
    pub menu_damping: f64,
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
            panel_content: "reveal".into(),
            bar_content: "slide".into(),
            open_speed: 1.0,
            close_speed: 1.0,
            hover_nudge: false,
            cascade: false,
            rolling: false,
            magnify: false,
            marker_glide: false,
            notification_nudge: false,
            bounce: false,
            menu_open: "expand".into(),
            menu_cascade: true,
            menu_stiffness: 775.0,
            menu_damping: 43.0,
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
    /// Round the monitors' corners with small black masks.
    pub rounded_corners: bool,
    /// Their radius (logical px).
    pub corner_radius: f64,
    /// Part of the bars (following their animations, tint and opacity), or
    /// detached: pure black, fixed in the screen corners.
    pub corners_attached: bool,
    /// Tooltips when hovering widgets.
    pub tooltips: bool,
    /// Other apps' tooltips (classic ones: Explorer's, the tray's) in Kasumi's look
    /// (native/apptips.rs; experimental).
    pub app_tooltips: bool,
    /// Look for a new version on GitHub when Kasumi starts, and say so (updater.rs).
    pub check_updates: bool,
    /// Install new versions from GitHub by itself (updater.rs).
    pub auto_update: bool,
    /// After Kasumi updates, a window shows what's new (settings.rs `startup`).
    pub update_notes: bool,
}

impl Default for General {
    fn default() -> Self {
        Self {
            hide_in_fullscreen: true,
            rounded_corners: false,
            corner_radius: 10.0,
            corners_attached: true,
            tooltips: true,
            app_tooltips: false,
            check_updates: true,
            auto_update: false,
            update_notes: true,
        }
    }
}

/// `[shell]`: Kasumi standing in for parts of the Windows shell.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Shell {
    /// Hide the Windows taskbar while Kasumi runs (restored when it quits).
    pub hide_taskbar: bool,
}

/// `[bar_menu]`: the menu on a bar's empty space (right-click).
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct BarMenuCfg {
    /// Lock, Sleep, Restart and Shut down in it (each but Lock asks for a second click).
    pub power: bool,
    /// How it's arranged (the context menu's layout format; empty: as Kasumi lists it).
    pub layout: Vec<MenuNode>,
}

/// `[launcher_menu]`: the menu on an app in the launcher (right-click): Windows' own
/// entries for it (as Start's) and Kasumi's.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct LauncherMenuCfg {
    /// How it's arranged (the context menu's layout format; empty: as Windows lists it).
    pub layout: Vec<MenuNode>,
}

/// `[context_menu]`: Kasumi's own right-click menus in Explorer and on the desktop
/// (experimental; native bars).
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct ContextMenuCfg {
    pub enabled: bool,
    /// Icons beside the entries.
    pub icons: bool,
    /// Tray icons' menus in Kasumi's look too (the apps' own Windows menus).
    #[serde(default = "default_true")]
    pub tray_apps: bool,
    /// How the menu is arranged (empty: Kasumi's default, native/ctxmenu/layout.rs).
    pub layout: Vec<MenuNode>,
    /// Your own entries and folders, where the layout says `custom = true`.
    pub custom: Vec<MenuNode>,
}

/// A place in a context menu's layout: one of Windows' entries (`match`), Kasumi's
/// own (`kasumi`), a command (`cmd`), a folder of more (`folder` + `items`), a
/// separator, or the rest of Windows' entries (`rest`).
#[derive(Debug, Clone, Deserialize, serde::Serialize, Default)]
#[serde(default)]
pub struct MenuNode {
    /// Windows' entries it stands for: `verb:copyaspath`, `text:Pin to*` (a trailing
    /// `*` matches the start).
    #[serde(rename = "match")]
    pub r#match: Vec<String>,
    /// Kasumi's own entries: "view", "terminal", "file_manage", "go_to".
    pub kasumi: String,
    pub folder: String,
    pub items: Vec<MenuNode>,
    /// Its name (renames a Windows entry; a command's text).
    pub name: String,
    /// A Segoe Fluent Icons code point, in hex ("E756").
    pub icon: String,
    /// A command: the program, and its arguments ({path}, {paths}, {dir}).
    pub cmd: String,
    pub args: String,
    /// Not shown (Windows' entries it matches are left out).
    pub hide: bool,
    pub separator: bool,
    pub rest: bool,
    pub custom: bool,
    /// Only with Shift held.
    pub shift: bool,
    /// Only for these: "file|folder|drive|multiple|background|desktop" (empty: all).
    #[serde(rename = "where")]
    pub r#where: String,
}

/// `[lang_switch]`: Kasumi's keyboard language picker (Win + Space held; native bars).
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct LangSwitchCfg {
    pub enabled: bool,
    /// Where it shows: "default" (where the switchers show: [alt_tab] position),
    /// "widget" (out of the language widget's notch; else the default), "top",
    /// "center" or "bottom".
    pub position: String,
    /// Each language: "both" (its code and name) or "codes".
    pub labels: String,
}

impl Default for LangSwitchCfg {
    fn default() -> Self {
        Self { enabled: true, position: "default".into(), labels: "both".into() }
    }
}

/// `[alt_tab]`: Kasumi's own Alt + Tab switcher (native bars), out of the top bar's notch.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct AltTabCfg {
    pub enabled: bool,
    /// Where it shows: "top" (out of the top bar's notch), "center" (a panel in the
    /// middle of the screen) or "bottom" (out of the dock's notch).
    pub position: String,
    /// A notch holding the tasks widget turns into the switcher (the apps step aside)
    /// instead of the switcher hanging under it.
    pub replace_tasks: bool,
    /// "current": the windows on the monitor you're working on; "all": every monitor's.
    pub monitors: String,
    /// Live previews of the windows.
    pub previews: bool,
    /// Under each window: "both" (icon and title), "icons", "titles", or "none"
    /// (previews only; without previews it shows both).
    pub labels: String,
    /// "Show desktop" as a choice: "off", "first" or "last" in the list.
    pub desktop: String,
}

impl Default for AltTabCfg {
    fn default() -> Self {
        Self { enabled: true, position: "top".into(), replace_tasks: true, monitors: "current".into(), previews: true, labels: "both".into(), desktop: "off".into() }
    }
}

/// `[launcher]`: the app launcher (a panel in a bar or dock).
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct LauncherCfg {
    pub enabled: bool,
    /// Global hotkey, e.g. "Win+Alt+Space"; empty for none.
    pub hotkey: String,
    /// The Windows key alone opens the launcher instead of the Start menu.
    pub start_menu: bool,
    /// The Windows key / hotkey over a fullscreen app: "launcher" opens it there;
    /// "notches" shows that screen's bars for a few seconds instead (pressed again
    /// while they show, it opens the launcher).
    pub fullscreen: String,
}

impl Default for LauncherCfg {
    fn default() -> Self {
        Self { enabled: true, hotkey: "Win+Alt+Space".into(), start_menu: false, fullscreen: "launcher".into() }
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
    /// Use Windows' accent colour (Settings > Personalization > Colors) instead of `accent`.
    pub accent_sync: bool,
    /// Text fonts, installed families ("" = the built-in one; icons keep theirs):
    /// main text (titles, the clock, values) and secondary text (small details:
    /// labels, times, descriptions; "" = like the main font).
    pub font: String,
    pub font_secondary: String,
    /// Hot readings (temps above their warning level).
    pub warn: String,
    /// Widget details in the accent colour: hover highlights, the task in front's
    /// line, sliders, bars, the weather panel's temperature... (each widget can
    /// differ: `[widgets.<w>] accent_details`).
    pub accent_details: bool,
    pub font_size: f64,
    /// Opacity of the bars, notches and panels (percent); text stays solid.
    pub opacity: f64,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            background: "#000000".into(),
            text: "#ffffff".into(),
            dim_opacity: 0.45,
            accent: "#ffffff".into(),
            accent_sync: false,
            font: String::new(),
            font_secondary: String::new(),
            warn: "#ff9f43".into(),
            accent_details: false,
            font_size: 13.0,
            opacity: 100.0,
        }
    }
}

impl Osd {
    /// Where a pop-up ("volume", "brightness", "caps", "keyboard", "notification")
    /// appears: (screen, spot), its own or the default.
    pub fn place(&self, kind: &str) -> (&str, &str) {
        let (screen, spot) = match kind {
            "volume" => (&self.volume_screen, &self.volume_spot),
            "brightness" => (&self.brightness_screen, &self.brightness_spot),
            "caps" => (&self.caps_screen, &self.caps_spot),
            "keyboard" => (&self.keyboard_screen, &self.keyboard_spot),
            "notification" => (&self.notification_screen, &self.notification_spot),
            _ => (&self.screen, &self.spot),
        };
        fn or<'a>(own: &'a str, default: &'a str) -> &'a str {
            if own.trim().is_empty() { default } else { own }
        }
        (or(screen, &self.screen), or(spot, &self.spot))
    }
}

/// On-screen display: a notch briefly grows into a pill showing a level.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, rename_all(serialize = "camelCase"))]
pub struct Osd {
    pub enabled: bool,
    /// Where pop-ups appear unless their own setting says otherwise. Screen:
    /// "cursor" (the monitor under it), "main", "all", "others" (all but the
    /// main one) or a monitor, matched like a [[monitors]] rule. Spot: a bar
    /// and its notch, "top-center", "top-left", ... "bottom-right" (left / right:
    /// the corner notches; a bar without one there uses its center notch).
    pub screen: String,
    pub spot: String,
    /// Per pop-up: "" = like `screen` / `spot`. The keyboard one's spot can also
    /// be "widget": a small notch at the language widget.
    pub volume_screen: String,
    pub volume_spot: String,
    pub brightness_screen: String,
    pub brightness_spot: String,
    pub caps_screen: String,
    pub caps_spot: String,
    pub keyboard_screen: String,
    pub keyboard_spot: String,
    pub notification_screen: String,
    pub notification_spot: String,
    pub width: f64,
    pub height: f64,
    pub duration_ms: u64,
    /// With the bar open: "inline" (the pop-up takes its notch's place in the bar's
    /// row; native bars) or "below" (a pill under the bar, like a panel).
    pub open_bar: String,
    /// Volume change per key press, in percent.
    pub volume_step: f64,
    /// Take over the volume keys (suppresses the Windows volume flyout).
    pub replace_volume_keys: bool,
    /// Show brightness changes of the built-in panel (Fn keys).
    pub brightness: bool,
    /// While Kasumi shows brightness changes, hide the Windows brightness flyout.
    pub hide_windows_brightness: bool,
    /// Pop a small notch at the language widget when the keyboard layout changes.
    pub keyboard: bool,
    /// Width of that keyboard-layout peek notch.
    pub keyboard_width: f64,
    /// Pop the OSD when Caps Lock is switched on or off.
    pub caps_lock: bool,
    /// Explorer's file copies and moves: their progress as a level pop-up (osd/fileops.rs).
    pub file_progress: bool,
    /// A scroll bar at the right edge of the screen while a window under the cursor
    /// scrolls (osd/scroll.rs): native windows only, not browsers.
    pub scroll: bool,
    /// Its length (px; at most 80% of the screen's height).
    pub scroll_length: f64,
    /// Its width: how far its notch stands out of the screen's edge (px).
    pub scroll_width: f64,
    /// Pop new notifications out of the notch (Windows' own store, see
    /// providers/notifications.rs).
    pub notifications: bool,
    /// "line" (app, title, text on one line), "two" (title over text) or
    /// "compact" (icon and app only).
    pub notification_style: String,
    /// How long a notification stays out.
    pub notification_ms: u64,
    /// The notification pop-up is as wide as its text (up to `notification_width`), else always that wide.
    pub notification_fit: bool,
    /// Clicking a notification pop-up: "app" (opens the app that sent it), "panel"
    /// (Kasumi's notifications panel), "center" (Windows' notification center) or
    /// "dismiss" (puts it away).
    pub notification_click: String,
    pub notification_width: f64,
}

impl Default for Osd {
    fn default() -> Self {
        Self {
            enabled: true,
            screen: "cursor".into(),
            spot: "top-center".into(),
            volume_screen: String::new(),
            volume_spot: String::new(),
            brightness_screen: String::new(),
            brightness_spot: String::new(),
            caps_screen: String::new(),
            caps_spot: String::new(),
            keyboard_screen: String::new(),
            keyboard_spot: "widget".into(),
            notification_screen: String::new(),
            notification_spot: String::new(),
            width: 230.0,
            height: 0.0,
            duration_ms: 1500,
            open_bar: "inline".into(),
            volume_step: 2.0,
            replace_volume_keys: true,
            brightness: true,
            hide_windows_brightness: true,
            keyboard: true,
            keyboard_width: 120.0,
            caps_lock: false,
            file_progress: false,
            scroll: false,
            scroll_length: 800.0,
            scroll_width: 10.0,
            notifications: false,
            notification_style: "line".into(),
            notification_ms: 5000,
            notification_fit: true,
            notification_click: "app".into(),
            notification_width: 420.0,
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

/// Which screen edge a surface hangs from.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Edge {
    #[default]
    Top,
    Bottom,
}

/// `bar`: notches open into the full-width bar. `dock`: notches only; they grow in
/// place. `always`: the full-width bar, always shown (no notches).
#[derive(Debug, Clone, Copy, Deserialize, Serialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SurfaceMode {
    #[default]
    Bar,
    Dock,
    Always,
}

/// A surface's hot zones: resting the cursor there opens it.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Hot {
    pub left: bool,
    pub right: bool,
    /// Anywhere along the surface's edge.
    pub edge: bool,
    /// Corner square size (logical px).
    pub corner: f64,
    /// Depth of the edge zone (logical px).
    pub edge_size: f64,
}

/// Named surface heights (`[sizes]`): tiny is a bare hover handle.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SizeName {
    Tiny,
    #[default]
    Small,
    Medium,
    Large,
}

/// `[sizes]`: logical px heights of the named sizes.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct Sizes {
    pub tiny: f64,
    pub small: f64,
    pub medium: f64,
    pub large: f64,
}

impl Default for Sizes {
    fn default() -> Self {
        Self { tiny: 12.0, small: 30.0, medium: 45.0, large: 60.0 }
    }
}

impl Sizes {
    pub fn px(&self, size: SizeName) -> f64 {
        match size {
            SizeName::Tiny => self.tiny,
            SizeName::Small => self.small,
            SizeName::Medium => self.medium,
            SizeName::Large => self.large,
        }
    }
}

/// `[surfaces.<id>]`: a bar or dock on every monitor (rules can override it).
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct SurfaceCfg {
    pub enabled: bool,
    pub edge: Edge,
    pub mode: SurfaceMode,
    /// Size while idle and while open (dock mode).
    pub rest: SizeName,
    pub open: SizeName,
    /// Dock: shrink to the tiny handle while a window covers it.
    pub shrink_when_covered: bool,
    /// Reserve the bar's space on screen (like the taskbar): maximized windows stop at it.
    pub reserve_space: bool,
    /// Hot zones (absent: the top bar falls back to the old [timing] switches).
    pub hot_left: Option<bool>,
    pub hot_right: Option<bool>,
    pub hot_edge: Option<bool>,
    pub hot_corner_size: Option<f64>,
    pub hot_edge_size: Option<f64>,
    pub left: Vec<String>,
    pub center: Vec<String>,
    pub right: Vec<String>,
    pub notch: Vec<String>,
}

impl Default for SurfaceCfg {
    fn default() -> Self {
        Self {
            enabled: true,
            edge: Edge::Top,
            mode: SurfaceMode::Bar,
            rest: SizeName::Small,
            open: SizeName::Small,
            shrink_when_covered: true,
            reserve_space: false,
            hot_left: None,
            hot_right: None,
            hot_edge: None,
            hot_corner_size: None,
            hot_edge_size: None,
            left: vec![],
            center: vec![],
            right: vec![],
            notch: vec![],
        }
    }
}

impl SurfaceCfg {
    pub fn layout(&self) -> LayoutCfg {
        LayoutCfg { left: self.left.clone(), center: self.center.clone(), right: self.right.clone(), notch: self.notch.clone() }
    }
}

/// A monitor rule's changes to one surface: switch it off, or give it its own lists.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct SurfaceOverride {
    pub enabled: Option<bool>,
    pub left: Option<Vec<String>>,
    pub center: Option<Vec<String>>,
    pub right: Option<Vec<String>>,
    pub notch: Option<Vec<String>>,
}

impl SurfaceOverride {
    /// The monitor's own layout, if it has one (any list set).
    pub fn layout(&self) -> Option<LayoutCfg> {
        if self.left.is_none() && self.center.is_none() && self.right.is_none() && self.notch.is_none() {
            return None;
        }
        Some(LayoutCfg {
            left: self.left.clone().unwrap_or_default(),
            center: self.center.clone().unwrap_or_default(),
            right: self.right.clone().unwrap_or_default(),
            notch: self.notch.clone().unwrap_or_default(),
        })
    }
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
    /// This monitor's own layout; absent = follow the global `[layout]`. (v3 model.)
    pub layout: Option<LayoutCfg>,
    /// Per-surface changes on this monitor.
    #[serde(default)]
    pub surfaces: HashMap<String, SurfaceOverride>,
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

    /// Whether a widget of this type (`type` or `type#n`) is placed on any surface.
    pub fn widget_placed(&self, ty: &str) -> bool {
        let is = |w: &String| w.split('#').next() == Some(ty) && self.widget_enabled(w);
        let surfaces = self.surfaces.iter().flat_map(|m| m.values()).flat_map(|s| [&s.left, &s.center, &s.right]);
        let layout = self.layout.iter().flat_map(|l| [&l.left, &l.center, &l.right]);
        let rules = self.monitors.iter().flat_map(|r| r.surfaces.values()).flat_map(|o| [&o.left, &o.center, &o.right]).flatten();
        surfaces.chain(layout).chain(rules).flatten().any(is)
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
        self.notches_and_bar(layout)
    }

    /// The surfaces every monitor gets, before monitor rules. Configs from before
    /// surfaces existed have exactly one: the top bar.
    pub fn surface_list(&self) -> Vec<(String, SurfaceCfg)> {
        match &self.surfaces {
            // The top bar hangs from the top edge and the bottom bar from the bottom one.
            Some(map) => map
                .iter()
                .map(|(id, s)| {
                    let mut s = s.clone();
                    match id.as_str() {
                        "top" => s.edge = Edge::Top,
                        "bottom" => s.edge = Edge::Bottom,
                        _ => {}
                    }
                    (id.clone(), s)
                })
                .collect(),
            None => vec![("top".into(), SurfaceCfg::default())],
        }
    }

    /// A surface's hot zones; the top bar falls back to the old `[timing]` switches.
    pub fn hot(&self, id: &str, s: &SurfaceCfg) -> Hot {
        let t = &self.timing;
        let legacy = |flag: Option<bool>, pos: &str| id == "top" && t.hot_corner_enabled && flag.unwrap_or(t.hot_corner_position == pos);
        Hot {
            left: s.hot_left.unwrap_or_else(|| legacy(t.hot_top_left, "top_left")),
            right: s.hot_right.unwrap_or_else(|| legacy(t.hot_top_right, "top_right")),
            edge: s.hot_edge.unwrap_or_else(|| legacy(t.hot_top_edge, "top_edge")),
            corner: s.hot_corner_size.unwrap_or(t.hot_corner),
            edge_size: s.hot_edge_size.unwrap_or(t.edge_px),
        }
    }

    /// Whether surface `id` shows on a monitor with this rule.
    pub fn surface_on(&self, rule: Option<&MonitorRule>, id: &str, s: &SurfaceCfg) -> bool {
        s.enabled && rule.and_then(|r| r.surfaces.get(id)).and_then(|o| o.enabled) != Some(false)
    }

    /// The notches and bar groups of surface `id` on a monitor with this rule.
    pub fn resolve_surface(&self, rule: Option<&MonitorRule>, id: &str) -> (Vec<Notch>, Bar) {
        let Some(map) = &self.surfaces else { return self.resolve_layout(rule) };
        let Some(s) = map.get(id) else {
            return (Vec::new(), Bar { left: vec![], center: vec![], right: vec![], ..self.bar.clone() });
        };
        let own = rule.and_then(|r| r.surfaces.get(id)).and_then(|o| o.layout());
        let mut layout = own.unwrap_or_else(|| s.layout());
        if s.mode == SurfaceMode::Always {
            // Always shown in full: nothing to collapse into notches.
            layout.notch.clear();
            let (_, bar) = self.notches_and_bar(&layout);
            return (Vec::new(), bar);
        }
        if s.mode == SurfaceMode::Dock {
            // A dock has no bar: every widget lives in a notch.
            layout.notch = [&layout.left, &layout.center, &layout.right].into_iter().flatten().cloned().collect();
            let (notches, mut bar) = self.notches_and_bar(&layout);
            bar.left.clear();
            bar.center.clear();
            bar.right.clear();
            return (notches, bar);
        }
        self.notches_and_bar(&layout)
    }

    fn notches_and_bar(&self, layout: &LayoutCfg) -> (Vec<Notch>, Bar) {
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
    PathBuf::from(std::env::var("APPDATA").unwrap_or_else(|_| ".".into())).join("Kasumi")
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
    fn default_config_is_top_bar_and_dock() {
        let cfg = fallback();
        assert_eq!(cfg.surface_list().len(), 2);
        assert!(cfg.shell.hide_taskbar);
        let (notches, bar) = cfg.resolve_surface(cfg.monitors.first(), "top");
        let ids: Vec<_> = notches.iter().map(|n| (n.id.as_str(), n.widgets.clone())).collect();
        assert_eq!(ids, [("center", vec!["clock".to_string()])]);
        assert!(notches[0].auto_width);
        // Shipped minimal (the Default preset): menu, clock and buttons on top,
        // tasks, tray and quick settings in the dock; the rest are opt-in.
        assert_eq!(bar.left, ["menu"]);
        assert_eq!(bar.center, ["clock"]);
        assert_eq!(bar.right, ["buttons"]);
        let (dock, _) = cfg.resolve_surface(cfg.monitors.first(), "bottom");
        let widgets: Vec<String> = dock.iter().flat_map(|n| n.widgets.clone()).collect();
        assert_eq!(widgets, ["tasks", "tray", "quick"]);
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
    fn surfaces_resolve_with_monitor_overrides() {
        let cfg: Config = toml::from_str(
            r#"
            [surfaces.top]
            left = ["menu"]
            center = ["clock"]
            notch = ["clock"]

            [surfaces.bottom]
            edge = "bottom"
            mode = "dock"
            rest = "tiny"
            open = "medium"
            center = ["tasks", "buttons#2"]
            notch = ["tasks", "buttons#2"]

            [[monitors]]
            match = "ARZOPA"
            surfaces = { bottom = { enabled = false }, top = { center = ["clock"], right = ["temps"], notch = [] } }
            "#,
        )
        .unwrap();
        let ids: Vec<_> = cfg.surface_list().into_iter().map(|(id, _)| id).collect();
        assert_eq!(ids, ["bottom", "top"]);
        let bottom = cfg.surfaces.as_ref().unwrap()["bottom"].clone();
        assert_eq!((bottom.edge, bottom.mode, bottom.rest, bottom.open), (Edge::Bottom, SurfaceMode::Dock, SizeName::Tiny, SizeName::Medium));

        let (notches, bar) = cfg.resolve_surface(None, "bottom");
        assert_eq!(notches[0].widgets, ["tasks", "buttons#2"]);
        assert!(bar.center.is_empty(), "a dock has no bar");

        // ARZOPA: no dock, and its own top layout.
        let rule = cfg.monitors.first();
        assert!(!cfg.surface_on(rule, "bottom", &bottom));
        let (notches, bar) = cfg.resolve_surface(rule, "top");
        assert!(notches.is_empty());
        assert_eq!((bar.left.len(), bar.right.clone()), (0, vec!["temps".to_string()]));
    }

    #[test]
    fn docks_show_every_widget_as_a_notch() {
        let cfg: Config = toml::from_str(
            r#"
            [surfaces.bottom]
            edge = "bottom"
            mode = "dock"
            left = ["menu"]
            center = ["clock", "tray"]
            notch = []
            "#,
        )
        .unwrap();
        let (notches, bar) = cfg.resolve_surface(None, "bottom");
        let ids: Vec<_> = notches.iter().map(|n| (n.id.as_str(), n.widgets.len())).collect();
        assert_eq!(ids, [("left", 1), ("center", 2)]);
        assert!(bar.left.is_empty() && bar.center.is_empty() && bar.right.is_empty());
        assert_eq!((cfg.sizes.px(SizeName::Medium), cfg.sizes.px(SizeName::Large)), (45.0, 60.0));
    }

    #[test]
    fn always_on_bars_have_no_notches_and_hot_zones_are_per_bar() {
        let cfg: Config = toml::from_str(
            r#"
            [timing]
            hot_top_left = false
            hot_top_right = true

            [surfaces.top]
            center = ["clock"]
            notch = ["clock"]

            [surfaces.bottom]
            mode = "always"
            reserve_space = true
            hot_edge = true
            hot_edge_size = 6
            center = ["tasks"]
            "#,
        )
        .unwrap();
        let (notches, bar) = cfg.resolve_surface(None, "bottom");
        assert!(notches.is_empty());
        assert_eq!(bar.center, ["tasks"]);
        let list = cfg.surface_list();
        let (_, bottom) = list.iter().find(|(id, _)| id == "bottom").unwrap();
        let (_, top) = list.iter().find(|(id, _)| id == "top").unwrap();
        assert_eq!(bottom.edge, Edge::Bottom, "the bottom bar hangs from the bottom edge");
        assert!(bottom.reserve_space);
        // The top bar without its own switches falls back to [timing]; others don't.
        let h = cfg.hot("top", top);
        assert_eq!((h.left, h.right, h.edge), (false, true, false));
        let h = cfg.hot("bottom", bottom);
        assert_eq!((h.left, h.right, h.edge, h.edge_size), (false, false, true, 6.0));
    }

    #[test]
    fn configs_without_surfaces_are_one_top_bar() {
        let cfg: Config = toml::from_str(
            r#"
            [layout]
            center = ["clock"]
            notch = ["clock"]
            "#,
        )
        .unwrap();
        let list = cfg.surface_list();
        assert_eq!(list.len(), 1);
        assert_eq!((list[0].0.as_str(), list[0].1.edge, list[0].1.mode), ("top", Edge::Top, SurfaceMode::Bar));
        let (notches, _) = cfg.resolve_surface(None, "top");
        assert_eq!(notches[0].widgets, ["clock"]);
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
