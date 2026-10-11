//! Komorebi workspaces (the tiling window manager), for the workspaces widget:
//! each monitor's workspaces, which one is in front and which have windows.
//! Komorebi pushes its state on every change to a named pipe it's asked to
//! connect to (`komorebic subscribe-pipe`), so nothing is polled; the first
//! state comes from `komorebic state`. Only while the widget is placed and
//! komorebi runs; without it the widget hides ("workspaces" = null) and this
//! tries again now and then.

use serde_json::{json, Value};
use std::os::windows::process::CommandExt;
use std::process::Command;
use std::time::Duration;
use tauri::AppHandle;
use windows::core::w;
use windows::Win32::Foundation::{CloseHandle, ERROR_PIPE_CONNECTED, INVALID_HANDLE_VALUE};
use windows::Win32::Storage::FileSystem::{ReadFile, PIPE_ACCESS_INBOUND};
use windows::Win32::System::Pipes::{ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE, PIPE_WAIT};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const PIPE: &str = "kasumi-komorebi";

fn komorebic(args: &[&str]) -> std::io::Result<std::process::Output> {
    Command::new("komorebic").args(args).creation_flags(CREATE_NO_WINDOW).output()
}

/// What the widget needs from komorebi's state: per monitor (named like ours,
/// "DISPLAY4"), its workspaces (name, how many windows) and the focused one.
fn summary(state: &Value) -> Option<Value> {
    let monitors = state.get("monitors")?;
    let list: Vec<Value> = monitors
        .get("elements")?
        .as_array()?
        .iter()
        .map(|m| {
            let ws = &m["workspaces"];
            let spaces: Vec<Value> = ws["elements"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .enumerate()
                        .map(|(i, w)| {
                            let count = |v: &Value| v.as_array().map_or(0, |a| a.len());
                            let windows = count(&w["containers"]["elements"])
                                + count(&w["floating_windows"])
                                + usize::from(!w["monocle_container"].is_null())
                                + usize::from(!w["maximized_window"].is_null());
                            let name = w["name"].as_str().map(str::to_string).unwrap_or_else(|| (i + 1).to_string());
                            json!({ "name": name, "windows": windows })
                        })
                        .collect()
                })
                .unwrap_or_default();
            json!({ "name": m["name"], "focused": ws["focused"], "workspaces": spaces })
        })
        .collect();
    Some(json!({ "focused": monitors["focused"], "monitors": list }))
}

fn publish(app: &AppHandle, state: Option<&Value>) {
    super::publish(app, "workspaces", state.and_then(summary).unwrap_or(Value::Null));
}

/// Reads komorebi's notifications (one JSON object after another) until the pipe
/// closes (komorebi quit or restarted).
fn listen(app: &AppHandle) {
    unsafe {
        let pipe = CreateNamedPipeW(w!(r"\\.\pipe\kasumi-komorebi"), PIPE_ACCESS_INBOUND, PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT, 1, 0, 1 << 20, 0, None);
        if pipe == INVALID_HANDLE_VALUE {
            return;
        }
        // Komorebi connects to it once asked to.
        if komorebic(&["subscribe-pipe", PIPE]).is_err() {
            let _ = CloseHandle(pipe);
            return;
        }
        if let Err(e) = ConnectNamedPipe(pipe, None) {
            if e.code() != ERROR_PIPE_CONNECTED.to_hresult() {
                let _ = CloseHandle(pipe);
                return;
            }
        }
        let mut pending: Vec<u8> = Vec::new();
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let mut read = 0u32;
            if ReadFile(pipe, Some(&mut buf), Some(&mut read), None).is_err() || read == 0 {
                break;
            }
            pending.extend_from_slice(&buf[..read as usize]);
            // Every whole notification so far; a cut-off one waits for the rest, garbage is dropped.
            let all = pending.len();
            let mut used = 0;
            let mut last: Option<Value> = None;
            let mut stream = serde_json::Deserializer::from_slice(&pending).into_iter::<Value>();
            loop {
                match stream.next() {
                    Some(Ok(v)) => {
                        used = stream.byte_offset();
                        last = Some(v);
                    }
                    Some(Err(e)) if !e.is_eof() => {
                        used = all;
                        break;
                    }
                    _ => break,
                }
            }
            pending.drain(..used);
            if let Some(n) = last {
                if super::placed(app, "workspaces") {
                    publish(app, Some(n.get("state").unwrap_or(&n)));
                }
            }
        }
        let _ = CloseHandle(pipe);
    }
}

pub fn start(app: &AppHandle) {
    let app = app.clone();
    std::thread::Builder::new()
        .name("kasumi-komorebi".into())
        .spawn(move || loop {
            if !super::placed(&app, "workspaces") {
                std::thread::sleep(Duration::from_secs(2));
                continue;
            }
            // The state now (the pipe speaks only on changes); none: komorebi isn't running.
            let state = komorebic(&["state"]).ok().filter(|o| o.status.success()).and_then(|o| serde_json::from_slice::<Value>(&o.stdout).ok());
            publish(&app, state.as_ref());
            if state.is_none() {
                std::thread::sleep(Duration::from_secs(10));
                continue;
            }
            listen(&app);
            std::thread::sleep(Duration::from_secs(2));
        })
        .expect("spawn komorebi thread");
}

/// Workspaces widget click: show workspace `workspace` on komorebi's monitor `monitor`.
#[tauri::command]
pub async fn komorebi_focus(monitor: usize, workspace: usize) -> Result<(), String> {
    komorebic(&["focus-monitor-workspace", &monitor.to_string(), &workspace.to_string()]).map_err(|e| e.to_string())?;
    Ok(())
}
