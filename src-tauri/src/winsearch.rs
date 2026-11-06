//! Windows Search's index for the launcher ([widgets.launcher] index): the files,
//! folders and shortcuts whose names match what's typed, as Start and Explorer find
//! them. Asked through the index's OLE DB provider (Search.CollatorDSO) on a thread of
//! its own that keeps the connection; the answers are posted as `launcher:found`.

use serde_json::{json, Value};
use std::mem::{offset_of, size_of, ManuallyDrop};
use std::sync::mpsc::{channel, Sender};
use std::sync::Mutex;
use std::time::Duration;
use windows::core::{IUnknown, Interface, GUID, HSTRING};
use windows::Win32::Foundation::E_FAIL;
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED};
use windows::Win32::System::Search::{
    IAccessor, ICommandText, IDBCreateCommand, IDBCreateSession, IDBInitialize, IDataInitialize, IRowset, DBACCESSOR_ROWDATA, DBBINDING,
    DBMEMOWNER_CLIENTOWNED, DBPARAMIO_NOTPARAM, DBPART_LENGTH, DBPART_STATUS, DBPART_VALUE, DBTYPE_WSTR, HACCESSOR, MSDAINITIALIZE,
};

/// How many the index is asked for (hidden folders' files are left out of them, and
/// the launcher shows its own few).
const TOP: usize = 40;

/// The SQL dialect OLE DB providers take by default (DBGUID_DEFAULT).
const DBGUID_DEFAULT: GUID = GUID::from_u128(0xc8b521fb_5cf3_11ce_ade5_00aa0044773d);

static ASK: Mutex<Option<Sender<String>>> = Mutex::new(None);

/// Ask the index about `q` (lower case, as the launcher searches it). Only the newest
/// of the questions waiting is answered.
pub fn ask(q: &str) {
    let mut ask = ASK.lock().unwrap();
    if ask.is_none() {
        let (tx, rx) = channel::<String>();
        let started = std::thread::Builder::new().name("kasumi-search".into()).spawn(move || {
            unsafe {
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            }
            let mut session: Option<IDBCreateCommand> = None;
            while let Ok(mut q) = rx.recv() {
                // A word being typed: wait for its next letters.
                std::thread::sleep(Duration::from_millis(60));
                while let Ok(newer) = rx.try_recv() {
                    q = newer;
                }
                let items = find(&mut session, &q);
                crate::native::post(crate::native::Msg::Data("launcher:found".into(), json!({ "q": q, "items": items })));
            }
        });
        if started.is_err() {
            return;
        }
        *ask = Some(tx);
    }
    if let Some(tx) = ask.as_ref() {
        let _ = tx.send(q.to_string());
    }
}

/// The matches as [{path, name, folder}]: hidden folders' contents (`.cache`,
/// `node_modules`) left out, shortcuts named without `.lnk`.
fn find(session: &mut Option<IDBCreateCommand>, q: &str) -> Vec<Value> {
    let Some(sql) = sql(q) else { return Vec::new() };
    if session.is_none() {
        *session = connect().map_err(|e| eprintln!("[kasumi] Windows Search unavailable: {e}")).ok();
    }
    let Some(s) = session.as_ref() else { return Vec::new() };
    let rows = match run(s, &sql) {
        Ok(rows) => rows,
        Err(e) => {
            // The service restarted: connect again next time.
            eprintln!("[kasumi] Windows Search query failed: {e}");
            *session = None;
            return Vec::new();
        }
    };
    rows.into_iter()
        .filter(|[path, _]| !hidden(path))
        .map(|[path, name]| {
            let shown = [".lnk", ".url", ".appref-ms"].iter().find_map(|x| name.strip_suffix(x)).unwrap_or(&name).to_string();
            json!({ "path": path, "name": shown, "folder": std::path::Path::new(&path).is_dir() })
        })
        .collect()
}

/// A path inside a folder people don't browse: a dot folder or node_modules.
fn hidden(path: &str) -> bool {
    let mut parts: Vec<&str> = path.split('\\').collect();
    parts.pop();
    parts.iter().any(|p| p.starts_with('.') || p.eq_ignore_ascii_case("node_modules") || p.eq_ignore_ascii_case("$Recycle.Bin"))
}

/// The query for names with a word starting with each typed word (None: nothing to ask).
fn sql(q: &str) -> Option<String> {
    let words: Vec<String> = q
        .split_whitespace()
        .map(|w| w.chars().filter(|c| !matches!(c, '"' | '\'' | '*' | '%' | '[' | ']')).collect::<String>())
        .filter(|w| !w.is_empty())
        .collect();
    if words.is_empty() {
        return None;
    }
    let terms = words.iter().map(|w| format!("\"{w}*\"")).collect::<Vec<_>>().join(" AND ");
    Some(format!(
        "SELECT TOP {TOP} System.ItemPathDisplay, System.ItemNameDisplay FROM SystemIndex \
         WHERE SCOPE='file:' AND CONTAINS(System.ItemNameDisplay, '{terms}') ORDER BY System.Search.Rank DESC"
    ))
}

fn connect() -> windows::core::Result<IDBCreateCommand> {
    unsafe {
        let init: IDataInitialize = CoCreateInstance(&MSDAINITIALIZE, None, CLSCTX_INPROC_SERVER)?;
        let mut source: Option<IUnknown> = None;
        let how = HSTRING::from("provider=Search.CollatorDSO.1;EXTENDED PROPERTIES=\"Application=Windows\"");
        init.GetDataSource(None, CLSCTX_INPROC_SERVER.0, &how, &IDBInitialize::IID, &mut source)?;
        let db: IDBInitialize = source.ok_or(windows::core::Error::from(E_FAIL))?.cast()?;
        db.Initialize()?;
        db.cast::<IDBCreateSession>()?.CreateSession(None, &IDBCreateCommand::IID)?.cast()
    }
}

/// Room for a column's text (UTF-16 units): longer paths come cut, and are left out.
const TEXT: usize = 1024;
const COLS: usize = 2;

#[repr(C)]
struct Col {
    status: u32,
    len: usize,
    text: [u16; TEXT],
}

#[repr(C)]
struct Row {
    cols: [Col; COLS],
}

/// Each row's columns as text.
fn run(session: &IDBCreateCommand, sql: &str) -> windows::core::Result<Vec<[String; COLS]>> {
    unsafe {
        let command: ICommandText = session.CreateCommand(None, &ICommandText::IID)?.cast()?;
        command.SetCommandText(&DBGUID_DEFAULT, &HSTRING::from(sql))?;
        let mut rowset: Option<IUnknown> = None;
        command.Execute(None, &IRowset::IID, None, None, Some(&mut rowset))?;
        let rowset: IRowset = rowset.ok_or(windows::core::Error::from(E_FAIL))?.cast()?;
        let accessor: IAccessor = rowset.cast()?;
        let bindings: Vec<DBBINDING> = (0..COLS)
            .map(|i| {
                let at = offset_of!(Row, cols) + i * size_of::<Col>();
                DBBINDING {
                    iOrdinal: i + 1,
                    obValue: at + offset_of!(Col, text),
                    obLength: at + offset_of!(Col, len),
                    obStatus: at + offset_of!(Col, status),
                    pTypeInfo: ManuallyDrop::new(None),
                    pObject: std::ptr::null_mut(),
                    pBindExt: std::ptr::null_mut(),
                    dwPart: (DBPART_VALUE.0 | DBPART_LENGTH.0 | DBPART_STATUS.0) as u32,
                    dwMemOwner: DBMEMOWNER_CLIENTOWNED.0 as u32,
                    eParamIO: DBPARAMIO_NOTPARAM.0 as u32,
                    cbMaxLen: TEXT * 2,
                    dwFlags: 0,
                    wType: DBTYPE_WSTR.0 as u16,
                    bPrecision: 0,
                    bScale: 0,
                }
            })
            .collect();
        let mut handle = HACCESSOR::default();
        accessor.CreateAccessor(DBACCESSOR_ROWDATA.0 as u32, bindings.len(), bindings.as_ptr(), size_of::<Row>(), &mut handle, None)?;
        let mut out = Vec::new();
        let mut row: Box<Row> = Box::new(std::mem::zeroed());
        loop {
            let mut rows = [0usize; 20];
            let mut at = rows.as_mut_ptr();
            let mut got = 0usize;
            let hr = (Interface::vtable(&rowset).GetNextRows)(rowset.as_raw(), 0, 0, rows.len() as isize, &mut got, &mut at);
            if hr.is_err() || got == 0 {
                break;
            }
            for &r in &rows[..got] {
                if rowset.GetData(r, handle, (&mut *row as *mut Row).cast()).is_err() {
                    continue;
                }
                // DBSTATUS_S_OK only (a cut or missing value is no use).
                if row.cols.iter().all(|c| c.status == 0) {
                    out.push(std::array::from_fn(|i| {
                        let c = &row.cols[i];
                        String::from_utf16_lossy(&c.text[..(c.len / 2).min(TEXT)])
                    }));
                }
            }
            let _ = rowset.ReleaseRows(got, rows.as_ptr(), std::ptr::null(), std::ptr::null_mut(), std::ptr::null_mut());
        }
        let _ = accessor.ReleaseAccessor(handle, None);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asks_for_each_word() {
        let q = sql("photo shop").unwrap();
        assert!(q.contains(r#"CONTAINS(System.ItemNameDisplay, '"photo*" AND "shop*"')"#), "{q}");
        // Quotes can't end the query early.
        assert!(sql("it's").unwrap().contains(r#"'"its*"'"#));
        assert_eq!(sql("  '\" "), None);
    }

    #[test]
    fn hidden_folders() {
        assert!(hidden(r"C:\Users\x\.cache\a\spot.js"));
        assert!(hidden(r"C:\p\node_modules\lucide\spot.js"));
        assert!(!hidden(r"C:\Users\x\Documents\.notes.txt"));
        assert!(!hidden(r"C:\Tools\Shortcuts\Spotify.lnk"));
    }

    /// Windows Search on this PC: `cargo test index_answers -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn index_answers() {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        let mut session = None;
        for q in ["spot", "photoshop", "design"] {
            let t = std::time::Instant::now();
            let items = find(&mut session, q);
            println!("{q}: {} in {} ms", items.len(), t.elapsed().as_millis());
            for i in items.iter().take(8) {
                println!("  {i}");
            }
        }
    }
}
