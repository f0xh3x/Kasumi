//! Actions the frontend may trigger (menu entries). Only this fixed set is allowed.

use std::os::windows::process::CommandExt;
use std::process::Command;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Expands `~` to the user profile and normalizes `/` to `\` (Explorer needs
/// backslashes). `shell:` targets pass through untouched.
fn resolve_target(path: &str) -> String {
    if path.starts_with("shell:") {
        return path.to_string();
    }
    let path = match path.strip_prefix('~') {
        Some(rest) => format!("{}{rest}", std::env::var("USERPROFILE").unwrap_or_default()),
        None => path.to_string(),
    };
    path.replace('/', "\\")
}

#[tauri::command]
pub async fn run_action(app: tauri::AppHandle, action: String, target: Option<String>) -> Result<(), String> {
    // Our overlay never takes focus, so let whatever we launch come to the front.
    crate::win::allow_foreground();
    let notepad = |path: std::path::PathBuf| Command::new("notepad.exe").arg(path).spawn();
    let result = match action.as_str() {
        "open" => {
            let target = target.ok_or("'open' needs a target")?;
            Command::new("explorer.exe").arg(resolve_target(&target)).spawn()
        }
        "restart" => Command::new("shutdown.exe").args(["/r", "/t", "0"]).creation_flags(CREATE_NO_WINDOW).spawn(),
        "shutdown" => Command::new("shutdown.exe").args(["/s", "/t", "0"]).creation_flags(CREATE_NO_WINDOW).spawn(),
        "sign_out" => Command::new("shutdown.exe").args(["/l"]).creation_flags(CREATE_NO_WINDOW).spawn(),
        "sleep" | "hibernate" => {
            let hibernate = action == "hibernate";
            // SetSuspendState blocks until resume; don't hold the command thread.
            std::thread::spawn(move || unsafe {
                let _ = windows::Win32::System::Power::SetSuspendState(hibernate, false, false);
            });
            return Ok(());
        }
        "lock" => {
            return unsafe { windows::Win32::System::Shutdown::LockWorkStation() }.map_err(|e| e.to_string());
        }
        "settings" => {
            crate::settings::open(&app);
            return Ok(());
        }
        "open_config" => notepad(crate::config::path()),
        "open_log" => notepad(crate::config::dir().join("statusnotch.log")),
        "open_config_folder" => Command::new("explorer.exe").arg(crate::config::dir()).spawn(),
        other => return Err(format!("unknown action '{other}'")),
    };
    result.map(|_| ()).map_err(|e| e.to_string())
}
