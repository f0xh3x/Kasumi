//! Updates from Kasumi's GitHub releases. Settings > About checks the latest
//! release (`update_check`) and installs it (`update_install`); with
//! `[general] auto_update` on, this checks a minute after start and then every
//! 6 hours and installs a newer version by itself. Installing = download the
//! release's setup over HTTPS and run it silently into this install's folder:
//! the installer asks the running Kasumi to quit (hooks.nsh), then Kasumi starts
//! again. Only an installed copy (an uninstall.exe beside it) updates itself.

use serde_json::{json, Value};
use std::os::windows::process::CommandExt;
use std::process::Command;
use std::time::Duration;
use tauri::{AppHandle, Manager};

/// The latest release's page redirects to its tag (no GitHub API: its 60 requests an
/// hour per PC run out quickly); the installer's address follows from the version.
const REPO: &str = "https://github.com/f0xh3x/Kasumi";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const DETACHED_PROCESS: u32 = 0x0000_0008;

struct Release {
    version: String,
    page: String,
    /// The installer asset: (file name, download URL).
    installer: Option<(String, String)>,
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new().timeout(Duration::from_secs(30)).user_agent("Kasumi-updater").build()
}

fn latest() -> Result<Release, String> {
    // .../releases/latest answers with a redirect to .../releases/tag/v0.4.1.
    let no_redirects = ureq::AgentBuilder::new().timeout(Duration::from_secs(30)).user_agent("Kasumi-updater").redirects(0).build();
    let resp = match no_redirects.get(&format!("{REPO}/releases/latest")).call() {
        Ok(r) => r,
        Err(ureq::Error::Status(404, _)) => return Err("No releases yet.".into()),
        Err(e) => return Err(format!("Couldn't reach GitHub ({e}).")),
    };
    let page = resp.header("location").unwrap_or("").to_string();
    let Some(tag) = page.rsplit_once("/tag/").map(|(_, t)| t.to_string()) else {
        return Err("No releases yet.".into());
    };
    let version = tag.trim_start_matches('v').to_string();
    // The installer the release workflow attaches (tools/release.py): Kasumi_<version>_x64-setup.exe.
    let name = format!("Kasumi_{version}_x64-setup.exe");
    let url = format!("{REPO}/releases/download/{tag}/{name}");
    Ok(Release { version, page, installer: Some((name, url)) })
}

/// "0.4.1" > "0.4.0" (numbers compared part by part; anything after a "-" ignored).
fn newer(candidate: &str, current: &str) -> bool {
    let parts = |s: &str| -> Vec<u64> { s.split('-').next().unwrap_or("").split('.').map(|p| p.parse().unwrap_or(0)).collect() };
    parts(candidate) > parts(current)
}

fn current() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Downloads the release's installer and runs it silently into this install, then
/// starts the new version. Returns once it's on its way (Kasumi is asked to quit).
fn install(r: &Release) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = exe.parent().ok_or("No install folder")?.to_path_buf();
    if !dir.join("uninstall.exe").exists() {
        return Err("Only an installed Kasumi can update itself.".into());
    }
    let (name, url) = r.installer.clone().ok_or("The release has no installer.")?;
    let tmp = std::env::temp_dir().join("Kasumi-update");
    std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    let setup = tmp.join(&name);
    eprintln!("[kasumi] update: downloading {url}");
    let resp = agent().get(&url).call().map_err(|e| format!("Download failed ({e})."))?;
    let mut file = std::fs::File::create(&setup).map_err(|e| e.to_string())?;
    let bytes = std::io::copy(&mut resp.into_reader(), &mut file).map_err(|e| format!("Download failed ({e})."))?;
    drop(file);
    eprintln!("[kasumi] update: {bytes} bytes; installing {} into {}", r.version, dir.display());
    // A small script waits for the silent install (which quits this Kasumi first),
    // then starts the new one. NSIS wants /D= last and unquoted.
    let script = tmp.join("update.cmd");
    let body = format!(
        "@echo off\r\nstart \"\" /wait \"{}\" /S /D={}\r\nstart \"\" \"{}\"\r\n",
        setup.display(),
        dir.display(),
        dir.join("kasumi.exe").display()
    );
    std::fs::write(&script, body).map_err(|e| e.to_string())?;
    Command::new("cmd")
        .arg("/c")
        .arg(&script)
        .current_dir(&tmp)
        .creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS)
        .spawn()
        .map_err(|e| format!("Couldn't start the installer ({e})."))?;
    Ok(())
}

/// A release's notes (Markdown), from GitHub's API: one request per version and
/// start (remembered), well within its 60 an hour.
#[tauri::command]
pub async fn release_notes(version: String) -> Result<String, String> {
    use std::collections::HashMap;
    use std::sync::Mutex;
    static CACHE: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);
    let version = version.trim().trim_start_matches('v').to_string();
    if let Some(text) = CACHE.lock().unwrap().as_ref().and_then(|c| c.get(&version)) {
        return Ok(text.clone());
    }
    let url = format!("https://api.github.com/repos/f0xh3x/Kasumi/releases/tags/v{version}");
    let resp = agent()
        .get(&url)
        .set("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| match e {
            ureq::Error::Status(404, _) => format!("No notes for version {version}."),
            e => format!("Couldn't reach GitHub ({e})."),
        })?;
    let v: Value = serde_json::from_str(&resp.into_string().map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let text = v["body"].as_str().unwrap_or("").to_string();
    CACHE.lock().unwrap().get_or_insert_with(HashMap::new).insert(version, text.clone());
    Ok(text)
}

/// Settings > About: the running and the latest version.
#[tauri::command]
pub async fn update_check() -> Result<Value, String> {
    let r = latest()?;
    Ok(json!({
        "current": current(),
        "latest": r.version,
        "available": newer(&r.version, current()) && r.installer.is_some(),
        "page": r.page,
    }))
}

/// Settings > About: install the latest release now.
#[tauri::command]
pub async fn update_install() -> Result<(), String> {
    let result = latest().and_then(|r| {
        if !newer(&r.version, current()) {
            return Err(format!("Already up to date ({}).", current()));
        }
        install(&r)
    });
    if let Err(e) = &result {
        eprintln!("[kasumi] update failed: {e}");
    }
    result
}

/// The OSD notch says so, in the accent colour, for 6 s.
fn announce(app: &AppHandle, version: &str) {
    eprintln!("[kasumi] version {version} is available");
    crate::osd::emit(app, "update", "notch://osd", json!({ "kind": "update", "level": 1.0, "muted": false, "text": "Update is available", "ms": 6000 }));
}

/// Shortly after start: `[general] check_updates` looks once and announces a new
/// version; `auto_update` installs it, and looks again every 6 hours.
pub fn start(app: &AppHandle) {
    let app = app.clone();
    std::thread::Builder::new()
        .name("kasumi-updater".into())
        .spawn(move || {
            std::thread::sleep(Duration::from_secs(15));
            // Test aid: KASUMI_TEST_UPDATE=1 shows the notice as if a new version were out.
            if std::env::var_os("KASUMI_TEST_UPDATE").is_some() {
                announce(&app, "test");
            }
            // Test aid: KASUMI_TEST_INSTALL_UPDATE=1 installs the latest release now (as "Update now").
            if std::env::var_os("KASUMI_TEST_INSTALL_UPDATE").is_some() {
                let _ = tauri::async_runtime::block_on(update_install());
            }
            let mut first = true;
            loop {
                let (check, auto) = {
                    let cfg = app.state::<crate::providers::SharedConfig>();
                    let g = &cfg.read().unwrap().general;
                    (g.check_updates, g.auto_update)
                };
                if auto || (first && check) {
                    match latest() {
                        Ok(r) if newer(&r.version, current()) && r.installer.is_some() => {
                            if auto {
                                if let Err(e) = install(&r) {
                                    eprintln!("[kasumi] auto update to {}: {e}", r.version);
                                    announce(&app, &r.version);
                                }
                            } else {
                                announce(&app, &r.version);
                            }
                        }
                        Ok(_) => {}
                        Err(e) => eprintln!("[kasumi] update check: {e}"),
                    }
                }
                first = false;
                std::thread::sleep(Duration::from_secs(6 * 3600));
            }
        })
        .expect("spawn updater thread");
}

#[cfg(test)]
mod tests {
    #[test]
    fn versions() {
        assert!(super::newer("0.4.1", "0.4.0"));
        assert!(super::newer("0.10.0", "0.9.9"));
        assert!(!super::newer("0.4.0", "0.4.0"));
        assert!(!super::newer("0.3.9", "0.4.0"));
    }

    /// The latest release on GitHub (needs the network): cargo test latest_release -- --ignored
    #[test]
    #[ignore]
    fn latest_release() {
        let r = super::latest().expect("latest release");
        let (name, url) = r.installer.expect("installer");
        println!("{} {} {name} {url}", r.version, r.page);
        assert!(!r.version.is_empty() && url.ends_with("-setup.exe"));
    }
}
