//! Presets: named copies of the whole config, kept in `%APPDATA%\StatusNotch\presets`.
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
            let current = fs::read(&p).is_ok_and(|b| normalized(&b) == current);
            Some(Preset { name, modified, current })
        })
        .collect();
    out.sort_by_key(|p| p.name.to_lowercase());
    Ok(out)
}

/// Saves the config in use as a preset (replacing one of the same name).
#[tauri::command]
pub async fn preset_save(name: String) -> Result<(), String> {
    let target = file(&name)?;
    fs::create_dir_all(dir()).map_err(|e| e.to_string())?;
    fs::copy(config::path(), target).map(|_| ()).map_err(|e| e.to_string())
}

/// Makes a preset the config in use. The setup it replaces is kept as
/// config.before-preset.toml, in case it wasn't saved.
#[tauri::command]
pub async fn preset_apply(name: String) -> Result<(), String> {
    let text = fs::read_to_string(file(&name)?).map_err(|e| e.to_string())?;
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
}
