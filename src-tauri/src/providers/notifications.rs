//! Notifications, read from Windows' own store
//! (`%LOCALAPPDATA%\Microsoft\Windows\Notifications\wpndatabase.db`, SQLite) through
//! Windows' SQLite (winsqlite3.dll, loaded when needed). Read-only, when the store's
//! files change (checked each second: `stamp`), and only while the notifications widget is
//! placed or new ones pop out of the notch ([osd] notifications): the widget gets
//! the list ("notifications" data), the notch the new one (`notch://notification`).
//! Windows only stores notifications while they're switched on; with Do not
//! disturb on it shows none itself, which leaves showing them to us.

use serde_json::{json, Value};
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};
use tauri::AppHandle;
use windows::core::{s, w};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

const SQLITE_OK: c_int = 0;
const SQLITE_ROW: c_int = 100;
const SQLITE_OPEN_READONLY: c_int = 0x1;
const SQLITE_OPEN_URI: c_int = 0x40;

type Open = unsafe extern "system" fn(*const c_char, *mut *mut c_void, c_int, *const c_char) -> c_int;
type Prepare = unsafe extern "system" fn(*mut c_void, *const c_char, c_int, *mut *mut c_void, *mut *const c_char) -> c_int;
type Step = unsafe extern "system" fn(*mut c_void) -> c_int;
type ColInt = unsafe extern "system" fn(*mut c_void, c_int) -> i64;
type ColPtr = unsafe extern "system" fn(*mut c_void, c_int) -> *const c_void;
type ColBytes = unsafe extern "system" fn(*mut c_void, c_int) -> c_int;
type Close = unsafe extern "system" fn(*mut c_void) -> c_int;

/// The few SQLite functions used, from winsqlite3.dll.
struct Sqlite {
    open: Open,
    prepare: Prepare,
    step: Step,
    int64: ColInt,
    text: ColPtr,
    blob: ColPtr,
    bytes: ColBytes,
    finalize: Close,
    close: Close,
}

impl Sqlite {
    fn load() -> Option<Self> {
        unsafe {
            let lib = LoadLibraryW(w!("winsqlite3.dll")).ok()?;
            macro_rules! f {
                ($name:expr) => {
                    std::mem::transmute(GetProcAddress(lib, $name)?)
                };
            }
            Some(Self {
                open: f!(s!("sqlite3_open_v2")),
                prepare: f!(s!("sqlite3_prepare_v2")),
                step: f!(s!("sqlite3_step")),
                int64: f!(s!("sqlite3_column_int64")),
                text: f!(s!("sqlite3_column_text")),
                blob: f!(s!("sqlite3_column_blob")),
                bytes: f!(s!("sqlite3_column_bytes")),
                finalize: f!(s!("sqlite3_finalize")),
                close: f!(s!("sqlite3_close")),
            })
        }
    }
}

pub struct Note {
    pub id: i64,
    pub app: String,
    /// The app's AppUserModelID (its icon and how to start it: shell:AppsFolder).
    pub aumid: String,
    pub title: String,
    pub body: String,
    /// Unix seconds.
    pub time: i64,
}

fn db_path() -> PathBuf {
    PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default()).join(r"Microsoft\Windows\Notifications\wpndatabase.db")
}

/// The toast notifications in the store (newest first, at most 30).
pub fn read(path: &std::path::Path) -> Option<Vec<Note>> {
    let sq = Sqlite::load()?;
    let uri = CString::new(format!("file:{}?mode=ro", path.to_string_lossy().replace('\\', "/"))).ok()?;
    let sql = CString::new(
        "SELECT n.Id, h.PrimaryId, n.ArrivalTime, n.Payload FROM Notification n \
         LEFT JOIN NotificationHandler h ON h.RecordId = n.HandlerId \
         WHERE n.Type = 'toast' ORDER BY n.ArrivalTime DESC LIMIT 30",
    )
    .ok()?;
    let mut out = Vec::new();
    unsafe {
        let mut db: *mut c_void = std::ptr::null_mut();
        if (sq.open)(uri.as_ptr(), &mut db, SQLITE_OPEN_READONLY | SQLITE_OPEN_URI, std::ptr::null()) != SQLITE_OK {
            if !db.is_null() {
                (sq.close)(db);
            }
            return None;
        }
        let mut stmt: *mut c_void = std::ptr::null_mut();
        if (sq.prepare)(db, sql.as_ptr(), -1, &mut stmt, std::ptr::null_mut()) == SQLITE_OK {
            while (sq.step)(stmt) == SQLITE_ROW {
                let id = (sq.int64)(stmt, 0);
                let handler = {
                    let p = (sq.text)(stmt, 1) as *const c_char;
                    if p.is_null() { String::new() } else { CStr::from_ptr(p).to_string_lossy().into_owned() }
                };
                let arrival = (sq.int64)(stmt, 2);
                let payload = {
                    let p = (sq.blob)(stmt, 3) as *const u8;
                    let n = (sq.bytes)(stmt, 3).max(0) as usize;
                    if p.is_null() { String::new() } else { String::from_utf8_lossy(std::slice::from_raw_parts(p, n)).into_owned() }
                };
                let texts = toast_texts(&payload);
                out.push(Note {
                    id,
                    app: app_name(&handler),
                    aumid: handler.clone(),
                    title: texts.first().cloned().unwrap_or_default(),
                    body: texts.iter().skip(1).cloned().collect::<Vec<_>>().join("\n"),
                    // FILETIME (100 ns since 1601) -> unix seconds.
                    time: arrival / 10_000_000 - 11_644_473_600,
                });
            }
            (sq.finalize)(stmt);
        }
        (sq.close)(db);
    }
    Some(out)
}

/// The `<text>` lines of a toast's XML.
pub fn toast_texts(xml: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(i) = rest.find("<text") {
        rest = &rest[i + 5..];
        // "<text>" or "<text attr=...>", not "<textbox".
        if !rest.starts_with('>') && !rest.starts_with(' ') && !rest.starts_with('/') {
            continue;
        }
        let Some(open_end) = rest.find('>') else { break };
        if rest[..open_end].ends_with('/') {
            rest = &rest[open_end + 1..];
            continue;
        }
        rest = &rest[open_end + 1..];
        let Some(close) = rest.find("</text>") else { break };
        let t = unescape(rest[..close].trim());
        if !t.is_empty() {
            out.push(t);
        }
        rest = &rest[close + 7..];
    }
    out
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

/// A readable app name from a notification handler id (an AppUserModelID).
pub fn app_name(id: &str) -> String {
    if id.contains('\\') {
        // "{guid}\WindowsPowerShell\v1.0\powershell.exe"
        return std::path::Path::new(id).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    }
    // "Microsoft.WindowsStore_8wekyb3d8bbwe!App" / "com.squirrel.Discord.Discord"
    let base = id.split('!').next().unwrap_or(id);
    let base = base.split('_').next().unwrap_or(base);
    base.rsplit('.').next().unwrap_or(base).to_string()
}

/// Arrival time of the newest notification the user has seen (opened the widget);
/// kept in a small file so a restart doesn't make everything new again.
static SEEN: Mutex<Option<i64>> = Mutex::new(None);

fn seen_file() -> PathBuf {
    crate::config::dir().join("notifications-seen")
}

fn seen() -> i64 {
    let mut s = SEEN.lock().unwrap();
    *s.get_or_insert_with(|| std::fs::read_to_string(seen_file()).ok().and_then(|t| t.trim().parse().ok()).unwrap_or(0))
}

static LATEST: Mutex<Vec<(i64, i64)>> = Mutex::new(Vec::new());

fn publish(app: &AppHandle, notes: &[Note]) {
    let seen = seen();
    let items: Vec<Value> = notes
        .iter()
        .map(|n| json!({ "id": n.id, "app": n.app, "aumid": n.aumid, "title": n.title, "body": n.body, "time": n.time }))
        .collect();
    let unseen = notes.iter().filter(|n| n.time > seen).count();
    super::publish(app, "notifications", json!({ "count": notes.len(), "unseen": unseen, "items": items }));
}

/// The store's files as they are now (size and last write of the database and
/// its write-ahead log): cheap to ask for, and it changes with every write. A
/// folder watch reports nothing here (the notification service keeps the files
/// open), hence this.
fn stamp(path: &std::path::Path) -> [Option<(u64, SystemTime)>; 2] {
    let of = |p: &std::path::Path| std::fs::metadata(p).ok().and_then(|m| Some((m.len(), m.modified().ok()?)));
    [of(path), of(&path.with_extension("db-wal"))]
}

/// A new notification out of the OSD notch, where notifications appear.
fn pop(app: &AppHandle, n: &Note) {
    crate::osd::emit(app, "notification", "notch://notification", json!({ "app": n.app, "aumid": n.aumid, "title": n.title, "body": n.body }));
}

pub fn start(app: &AppHandle) {
    let app = app.clone();
    std::thread::Builder::new()
        .name("statusnotch-notifications".into())
        .spawn(move || {
            let path = db_path();
            // The newest notification already handled: only ones after it pop out
            // (None: not read yet, so whatever is there at the start is old).
            let mut newest: Option<i64> = None;
            let mut last = None;
            loop {
                let now_stamp = stamp(&path);
                let changed = last != Some(now_stamp);
                let list = super::placed(&app, "notifications");
                let popping = {
                    use tauri::Manager;
                    let cfg = app.state::<super::SharedConfig>();
                    let cfg = cfg.read().unwrap();
                    cfg.osd.enabled && cfg.osd.notifications
                };
                if !list && !popping {
                    newest = None;
                    last = None;
                } else if changed {
                    last = Some(now_stamp);
                    let notes = read(&path).unwrap_or_default();
                    *LATEST.lock().unwrap() = notes.iter().map(|n| (n.id, n.time)).collect();
                    if list {
                        publish(&app, &notes);
                    }
                    let top = notes.iter().map(|n| n.id).max();
                    if let (Some(prev), true) = (newest, popping) {
                        // The newest new one, if it just arrived (not an old one the store rewrote).
                        let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
                        if let Some(n) = notes.iter().filter(|n| n.id > prev && now - n.time < 60).max_by_key(|n| n.id) {
                            pop(&app, n);
                        }
                    }
                    // Read once: from now on anything newer is new (an empty store too).
                    newest = Some(top.unwrap_or(0).max(newest.unwrap_or(0)));
                }
                std::thread::sleep(Duration::from_secs(1));
            }
        })
        .expect("spawn notifications thread");
}

/// The user looked: what's there now isn't new any more.
#[tauri::command]
pub async fn notifications_seen(app: AppHandle) -> Result<(), String> {
    let newest = LATEST.lock().unwrap().iter().map(|(_, t)| *t).max().unwrap_or(0);
    *SEEN.lock().unwrap() = Some(newest.max(seen()));
    let _ = std::fs::write(seen_file(), newest.to_string());
    let notes = read(&db_path()).unwrap_or_default();
    publish(&app, &notes);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toast_text_lines() {
        let xml = r#"<toast><visual><binding template="ToastGeneric"><text hint-maxLines="1">Mia</text><text>Lunch? &amp; then a walk</text><text/><textbox id="r"/></binding></visual></toast>"#;
        assert_eq!(toast_texts(xml), ["Mia", "Lunch? & then a walk"]);
    }

    #[test]
    fn app_names() {
        assert_eq!(app_name(r"{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\WindowsPowerShell\v1.0\powershell.exe"), "powershell");
        assert_eq!(app_name("Microsoft.WindowsStore_8wekyb3d8bbwe!App"), "WindowsStore");
        assert_eq!(app_name("com.squirrel.Discord.Discord"), "Discord");
    }

    /// Reads a database shaped like Windows' store through winsqlite3.dll (made by
    /// .scratch/make_wpn_test_db.py; run by hand with `cargo test notifications -- --ignored`).
    #[test]
    #[ignore]
    fn reads_a_store_shaped_database() {
        let p = std::path::PathBuf::from(std::env::var("WPN_TEST_DB").expect("set WPN_TEST_DB"));
        let notes = read(&p).expect("readable");
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[0].title, "Newer");
        assert_eq!(notes[0].app, "Discord");
        assert_eq!(notes[1].body, "Second line");
    }
}
