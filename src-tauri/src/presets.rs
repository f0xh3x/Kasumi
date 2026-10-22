//! Presets: named copies of the whole config, kept in `%APPDATA%\Kasumi\presets`.
//! Applying one writes it over config.toml, which the engine hot-reloads.

use std::fs;
use std::path::PathBuf;
use std::time::UNIX_EPOCH;

use serde::Serialize;

use crate::config;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preset {
    name: String,
    /// Last saved, ms since the Unix epoch.
    modified: u64,
    /// Same as the config in use.
    current: bool,
}

fn dir() -> PathBuf {
    config::dir().join("presets")
}

/// Presets that come with Kasumi (made from the author's, minus personal bits).
const SHIPPED: &[(&str, &str)] = &[
    ("Default", include_str!("../presets/Default.toml")),
    ("Modern", include_str!("../presets/Modern.toml")),
    ("Classic status bar", include_str!("../presets/Classic status bar.toml")),
    ("Classic taskbar", include_str!("../presets/Classic taskbar.toml")),
];

/// The shipped presets, for the first-time setup's cards: each bar's style
/// ("bar" | "dock" | "always" | "off").
#[tauri::command]
pub async fn presets_shipped() -> Vec<serde_json::Value> {
    SHIPPED
        .iter()
        .map(|(name, text)| {
            let cfg: toml::Value = toml::from_str(text).unwrap_or(toml::Value::Table(Default::default()));
            let style = |s: &str| {
                let t = cfg.get("surfaces").and_then(|v| v.get(s));
                if t.and_then(|t| t.get("enabled")).and_then(|v| v.as_bool()) == Some(false) {
                    "off".to_string()
                } else {
                    t.and_then(|t| t.get("mode")).and_then(|v| v.as_str()).unwrap_or("bar").to_string()
                }
            };
            serde_json::json!({ "name": name, "top": style("top"), "bottom": style("bottom") })
        })
        .collect()
}

/// Adds the shipped presets that weren't added before (once each: one the user
/// deleted or changed stays that way). Listed in `presets/.shipped`.
pub fn install_shipped() {
    let marker = dir().join(".shipped");
    let done = fs::read_to_string(&marker).unwrap_or_default();
    let mut added: Vec<&str> = done.lines().collect();
    let mut changed = false;
    for (name, text) in SHIPPED {
        if added.contains(name) {
            continue;
        }
        let path = dir().join(format!("{name}.toml"));
        if !path.exists() {
            if fs::create_dir_all(dir()).is_err() || fs::write(&path, text).is_err() {
                continue;
            }
            eprintln!("[kasumi] added preset {name}");
        }
        added.push(name);
        changed = true;
    }
    if changed {
        let _ = fs::write(marker, added.join("
"));
    }
}

/// Settings that are yours rather than a setup's: when a preset doesn't set them
/// (shipped ones don't), applying it keeps the ones in use.
const PERSONAL: &[(&str, &str)] =
    &[("tasks", "pins"), ("weather", "location"), ("weather", "latitude"), ("weather", "longitude")];

/// A preset's text with the personal settings it lacks taken from `current`.
fn with_personal(preset: &str, current: &str) -> String {
    use toml_edit::DocumentMut;
    let (Ok(mut doc), Ok(cur)) = (preset.parse::<DocumentMut>(), current.parse::<DocumentMut>()) else {
        return preset.to_string();
    };
    let mut changed = false;
    for (widget, key) in PERSONAL {
        let Some(value) = cur.get("widgets").and_then(|w| w.get(widget)).and_then(|t| t.get(key)) else { continue };
        if doc.get("widgets").and_then(|w| w.get(widget)).and_then(|t| t.get(key)).is_some() {
            continue;
        }
        doc["widgets"][widget][key] = value.clone();
        changed = true;
    }
    if changed { doc.to_string() } else { preset.to_string() }
}

/// The file for a preset name; the name becomes the file name, so it must be one.
fn file(name: &str) -> Result<PathBuf, String> {
    let name = name.trim();
    let bad = |c: char| c.is_control() || r#"<>:"/\|?*"#.contains(c);
    if name.is_empty() || name.chars().count() > 64 || name.chars().any(bad) || name.starts_with('.') || name.ends_with('.') {
        return Err(r#"Use a short name without < > : " / \ | ? *"#.into());
    }
    Ok(dir().join(format!("{name}.toml")))
}

/// Config text compared regardless of line endings.
fn normalized(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

#[tauri::command]
pub async fn presets_list() -> Result<Vec<Preset>, String> {
    let current = normalized(&fs::read(config::path()).unwrap_or_default());
    let Ok(entries) = fs::read_dir(dir()) else { return Ok(Vec::new()) };
    // In use = what applying it would give (with the personal settings kept).
    let same = |bytes: &[u8]| normalized(with_personal(&normalized(bytes), &current).as_bytes()) == current;
    let mut out: Vec<Preset> = entries
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            if p.extension()?.to_str()? != "toml" {
                return None;
            }
            let name = p.file_stem()?.to_str()?.to_string();
            let modified = e
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_millis() as u64);
            let current = fs::read(&p).is_ok_and(|b| same(&b));
            Some(Preset { name, modified, current })
        })
        .collect();
    out.sort_by_key(|p| p.name.to_lowercase());
    Ok(out)
}

/// The presets' names, and which is in use (the bar menu).
pub fn names() -> Vec<(String, bool)> {
    tauri::async_runtime::block_on(presets_list()).unwrap_or_default().into_iter().map(|p| (p.name, p.current)).collect()
}

/// Saves the config in use as a preset (replacing one of the same name).
#[tauri::command]
pub async fn preset_save(name: String) -> Result<(), String> {
    let target = file(&name)?;
    fs::create_dir_all(dir()).map_err(|e| e.to_string())?;
    fs::copy(config::path(), target).map(|_| ()).map_err(|e| e.to_string())
}

/// Makes a preset the config in use (keeping your pins and weather city if it
/// has none). The setup it replaces is kept as config.before-preset.toml.
#[tauri::command]
pub async fn preset_apply(name: String) -> Result<(), String> {
    // A shipped one that was deleted: as it came.
    let text = match fs::read_to_string(file(&name)?) {
        Ok(t) => t,
        Err(e) => SHIPPED.iter().find(|(n, _)| *n == name.trim()).map(|(_, t)| t.to_string()).ok_or(e.to_string())?,
    };
    let text = with_personal(&text, &fs::read_to_string(config::path()).unwrap_or_default());
    toml::from_str::<config::Config>(&text).map_err(|e| format!("This preset isn't a valid config: {e}"))?;
    let path = config::path();
    let _ = fs::copy(&path, config::dir().join("config.before-preset.toml"));
    fs::write(path, text).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn preset_rename(from: String, to: String) -> Result<(), String> {
    let (src, dst) = (file(&from)?, file(&to)?);
    // Windows file names ignore case: renaming "work" to "Work" is fine.
    if dst.exists() && from.trim().to_lowercase() != to.trim().to_lowercase() {
        return Err(format!("There's already a preset called “{}”", to.trim()));
    }
    fs::rename(src, dst).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn preset_delete(name: String) -> Result<(), String> {
    fs::remove_file(file(&name)?).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_must_be_file_names() {
        assert!(file("Gaming").is_ok());
        assert!(file("Work · 2 monitors").is_ok());
        for bad in ["", "  ", "a/b", r"a\b", "con:", "what?", ".hidden", "trailing."] {
            assert!(file(bad).is_err(), "{bad:?} should be rejected");
        }
        assert_eq!(file("  Gaming ").unwrap().file_name().unwrap(), "Gaming.toml");
    }

    #[test]
    fn shipped_presets_are_valid_and_impersonal() {
        for (name, text) in SHIPPED {
            let cfg: config::Config = toml::from_str(text).unwrap_or_else(|e| panic!("{name}: {e}"));
            let raw: toml::Table = toml::from_str(text).unwrap();
            let widgets = raw.get("widgets").and_then(|w| w.as_table());
            assert!(widgets.and_then(|w| w.get("tasks")).and_then(|t| t.get("pins")).is_none(), "{name} has pins");
            assert!(widgets.and_then(|w| w.get("weather")).and_then(|t| t.get("location")).is_none(), "{name} has a city");
            assert!(cfg.monitors.iter().all(|m| m.matcher == "*"), "{name} has monitor rules");
        }
    }

    #[test]
    fn applying_keeps_personal_settings_a_preset_lacks() {
        let current = "[widgets.tasks]
pins = [\"a.exe\"]

[widgets.weather]
location = \"Kaunas\"
latitude = 54.9
longitude = 23.9
";
        let preset = "[widgets.weather]
units = \"metric\"
";
        let out: toml::Table = toml::from_str(&with_personal(preset, current)).unwrap();
        assert_eq!(out["widgets"]["tasks"]["pins"][0].as_str(), Some("a.exe"));
        assert_eq!(out["widgets"]["weather"]["location"].as_str(), Some("Kaunas"));
        assert_eq!(out["widgets"]["weather"]["units"].as_str(), Some("metric"));
        // A preset that sets them wins.
        let own = "[widgets.tasks]
pins = []
";
        let out: toml::Table = toml::from_str(&with_personal(own, current)).unwrap();
        assert_eq!(out["widgets"]["tasks"]["pins"].as_array().map(|a| a.len()), Some(0));
    }
}
