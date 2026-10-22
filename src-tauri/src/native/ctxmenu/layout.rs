//! How the menu is arranged: Windows' entries and Kasumi's own, moved into folders,
//! renamed, hidden or reordered by a layout ([context_menu] layout; Kasumi's default
//! follows a Nilesoft Shell setup: Pin/Unpin and More options folders, Terminal,
//! File manage, Go to). Also runs Kasumi's own entries.

use windows::core::{HSTRING, PCWSTR};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

use super::shell::{self, Action, Entry, Icon, Sub, Target, ViewCmd};
use crate::config::MenuNode;

/// What was right-clicked.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    File,
    Folder,
    Drive,
    Multiple,
    /// A folder's empty space.
    Background,
    /// The desktop's empty space.
    Desktop,
}

impl Kind {
    /// Its name in a layout's `where` ("file|folder|drive|multiple|background|desktop").
    pub fn name(self) -> &'static str {
        match self {
            Kind::File => "file",
            Kind::Folder => "folder",
            Kind::Drive => "drive",
            Kind::Multiple => "multiple",
            Kind::Background => "background",
            Kind::Desktop => "desktop",
        }
    }

    fn selection(self) -> bool {
        !matches!(self, Kind::Background | Kind::Desktop)
    }
}

/// The click: what it was on, the paths selected, the folder shown, and where a
/// terminal opens ("here": the folder clicked, else the one shown).
pub struct Ctx {
    pub kind: Kind,
    pub paths: Vec<String>,
    pub dir: String,
    pub here: String,
    pub shift: bool,
}

/// Kasumi's own commands.
#[derive(Clone, PartialEq, Debug)]
pub enum Cmd {
    /// Starts a program (in `here`).
    Run(String, String),
    /// Opens a folder, file or URI.
    Open(String),
    /// Puts text on the clipboard.
    Copy(String),
    /// Select all (0), invert (1), none (2).
    Select(u8),
    /// Explorer's hidden files / file name extensions shown.
    ShowHidden,
    ShowExtensions,
    /// Turns a file attribute on or off for the selection.
    Attribute(u32),
    NewFolder,
    NewFile(&'static str),
}

const FILE_ATTRIBUTE_READONLY: u32 = 0x1;
const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
const FILE_ATTRIBUTE_ARCHIVE: u32 = 0x20;

/// Kasumi's default layout ([context_menu] layout's format).
const DEFAULT: &str = r#"
[[layout]]
kasumi = "view"
[[layout]]
separator = true
[[layout]]
rest = true
[[layout]]
separator = true
[[layout]]
folder = "Pin/Unpin"
icon = "E718"
items = [
  { match = ["verb:pintohome", "verb:pintostartscreen", "verb:taskbarpin", "text:Pin to*"] },
  { match = ["verb:unpinfromhome", "verb:unpinfromstartscreen", "verb:taskbarunpin", "text:Unpin*"] },
]
[[layout]]
folder = "More options"
icon = "E712"
items = [
  { match = ["text:Send to"] },
  { match = ["verb:Windows.ModernShare", "text:Share"] },
  { match = ["verb:link", "text:Create shortcut"] },
  { match = ["verb:setdesktopwallpaper", "text:Set as desktop background"] },
  { match = ["text:Rotate right"] },
  { match = ["text:Rotate left"] },
  { match = ["text:Map network drive*"] },
  { match = ["text:Disconnect network drive*", "text:Disconnect"] },
  { match = ["verb:format", "text:Format*"] },
  { match = ["verb:eject", "text:Eject"] },
  { match = ["text:Give access to"] },
  { match = ["text:Include in library"] },
  { match = ["verb:print", "text:Print"] },
]
[[layout]]
separator = true
[[layout]]
folder = "Terminal"
icon = "E756"
items = [
  { kasumi = "terminal" },
  { separator = true },
  { match = ["text:Open in Terminal", "text:Open Linux shell here", "text:Open PowerShell window here"] },
]
[[layout]]
folder = "File manage"
icon = "E8B7"
items = [
  { kasumi = "file_manage" },
  { match = ["verb:copyaspath", "text:Copy as path"] },
  { match = ["text:Customize this folder*"] },
]
[[layout]]
custom = true
[[layout]]
folder = "Go to"
icon = "E8AD"
where = "file|folder|drive|multiple"
items = [{ kasumi = "go_to" }]
[[layout]]
separator = true
[[layout]]
match = ["verb:properties"]
[[layout]]
hide = true
match = ["text:Restore previous versions", "text:Cast to Device", "text:AMD Software*", "text:File name extensions"]
"#;

#[derive(serde::Deserialize)]
struct DefaultLayout {
    layout: Vec<MenuNode>,
}

pub fn default_layout() -> Vec<MenuNode> {
    toml::from_str::<DefaultLayout>(DEFAULT).map(|d| d.layout).unwrap_or_default()
}

/// `text:Pin to*` / `verb:copyaspath`: does a Windows entry match?
fn matches(m: &str, e: &Entry) -> bool {
    let (kind, pat) = m.split_once(':').unwrap_or(("text", m));
    let hay = if kind == "verb" { e.verb.as_str() } else { e.text.as_str() };
    if hay.is_empty() {
        return false;
    }
    let (hay, pat) = (hay.to_lowercase(), pat.to_lowercase());
    match pat.strip_suffix('*') {
        Some(prefix) => hay.starts_with(prefix),
        None => hay == pat,
    }
}

fn applies(n: &MenuNode, ctx: &Ctx) -> bool {
    (!n.shift || ctx.shift) && (n.r#where.trim().is_empty() || n.r#where.split('|').any(|w| w.trim() == ctx.kind.name()))
}

/// Windows' entries a layout names anywhere (placed or hidden): "rest" leaves them out.
fn claimed(nodes: &[MenuNode], windows: &[Entry], used: &mut [bool]) {
    for n in nodes {
        if !n.r#match.is_empty() {
            for (i, e) in windows.iter().enumerate() {
                if n.r#match.iter().any(|m| matches(m, e)) {
                    used[i] = true;
                }
            }
        }
        claimed(&n.items, windows, used);
    }
}

fn glyph(hex: &str) -> Option<Icon> {
    u32::from_str_radix(hex.trim().trim_start_matches("0x"), 16).ok().and_then(char::from_u32).map(Icon::Glyph)
}

/// The menu: Windows' entries arranged by the layout (Kasumi's default unless the
/// config has its own), with the config's own entries where it says `custom`.
pub fn arrange(cfg: &crate::config::ContextMenuCfg, windows: Vec<Entry>, ctx: &Ctx, t: &Target) -> Vec<Entry> {
    let layout = if cfg.layout.is_empty() { default_layout() } else { cfg.layout.clone() };
    let mut used = vec![false; windows.len()];
    claimed(&layout, &windows, &mut used);
    claimed(&cfg.custom, &windows, &mut used);
    let mut out = place(&layout, &windows, &used, ctx, t, &cfg.custom);
    shell::tidy(&mut out);
    out
}

fn place(nodes: &[MenuNode], windows: &[Entry], used: &[bool], ctx: &Ctx, t: &Target, custom: &[MenuNode]) -> Vec<Entry> {
    let mut out = Vec::new();
    for n in nodes {
        if !applies(n, ctx) || n.hide {
            continue;
        }
        if n.separator {
            out.push(Entry::separator());
        } else if n.rest {
            out.extend(windows.iter().zip(used).filter(|(_, u)| !**u).map(|(e, _)| e.clone()));
        } else if n.custom {
            out.extend(place(custom, windows, used, ctx, t, &[]));
        } else if !n.folder.is_empty() {
            let mut items = place(&n.items, windows, used, ctx, t, custom);
            shell::tidy(&mut items);
            if !items.is_empty() {
                let mut e = Entry::named(&n.folder, Action::None, glyph(&n.icon));
                e.sub = Some(Sub::Ready(items));
                out.push(e);
            }
        } else if !n.kasumi.is_empty() {
            out.extend(builtin(&n.kasumi, ctx, t));
        } else if !n.cmd.is_empty() {
            out.push(Entry::named(&n.name, Action::Kasumi(Cmd::Run(n.cmd.clone(), fill(&n.args, ctx))), glyph(&n.icon)));
        } else if !n.r#match.is_empty() {
            for e in windows.iter().filter(|e| n.r#match.iter().any(|m| matches(m, e))) {
                let mut e = e.clone();
                if !n.name.is_empty() {
                    e.text = n.name.clone();
                }
                if let Some(g) = glyph(&n.icon) {
                    e.icon = Some(g);
                }
                out.push(e);
            }
        }
    }
    out
}

/// `{path}` (the first selected, else the folder), `{paths}` (all, quoted), `{dir}`.
fn fill(args: &str, ctx: &Ctx) -> String {
    let first = ctx.paths.first().cloned().unwrap_or_else(|| ctx.here.clone());
    let all = if ctx.paths.is_empty() { format!("\"{}\"", ctx.here) } else { ctx.paths.iter().map(|p| format!("\"{p}\"")).collect::<Vec<_>>().join(" ") };
    args.replace("{paths}", &all).replace("{path}", &first).replace("{dir}", &ctx.here)
}

fn item(text: &str, cmd: Cmd, g: Option<char>) -> Entry {
    Entry::named(text, Action::Kasumi(cmd), g.map(Icon::Glyph))
}

fn folder(text: &str, g: char, items: Vec<Entry>) -> Entry {
    let mut e = Entry::named(text, Action::None, Some(Icon::Glyph(g)));
    e.sub = Some(Sub::Ready(items));
    e
}

fn check(mut e: Entry, on: bool) -> Entry {
    e.checked = on;
    e
}

/// Kasumi's own entries by name: "view", "terminal", "file_manage", "go_to".
fn builtin(id: &str, ctx: &Ctx, t: &Target) -> Vec<Entry> {
    match id {
        "view" if !ctx.kind.selection() => shell::view_entries(t),
        "terminal" => terminal(),
        "file_manage" => file_manage(ctx),
        "go_to" => go_to(),
        _ => Vec::new(),
    }
}

fn terminal() -> Vec<Entry> {
    let mut v = vec![
        item("Command Prompt", Cmd::Run("cmd.exe".into(), String::new()), Some('\u{E756}')),
        item("PowerShell", Cmd::Run("powershell.exe".into(), "-NoExit".into()), Some('\u{E756}')),
    ];
    let wt = std::env::var("LOCALAPPDATA").map(|d| std::path::Path::new(&d).join(r"Microsoft\WindowsApps\wt.exe"));
    if wt.is_ok_and(|p| p.exists()) {
        v.push(item("Windows Terminal", Cmd::Run("wt.exe".into(), "-d .".into()), Some('\u{E756}')));
    }
    v
}

fn attributes(path: &str) -> u32 {
    let w = HSTRING::from(path);
    let a = unsafe { windows::Win32::Storage::FileSystem::GetFileAttributesW(PCWSTR(w.as_ptr())) };
    if a == u32::MAX {
        0
    } else {
        a
    }
}

fn explorer_flag(name: &str) -> Option<u32> {
    crate::win::reg_dword(windows::Win32::System::Registry::HKEY_CURRENT_USER, r"Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced", name)
}

fn file_manage(ctx: &Ctx) -> Vec<Entry> {
    let mut v = Vec::new();
    let p = std::path::Path::new;
    if ctx.kind.selection() && !ctx.paths.is_empty() {
        let mut copy = Vec::new();
        if ctx.paths.len() > 1 {
            copy.push(item(&format!("Copy {} paths", ctx.paths.len()), Cmd::Copy(ctx.paths.join("\r\n")), Some('\u{E8C8}')));
        } else {
            let path = &ctx.paths[0];
            copy.push(item(path, Cmd::Copy(path.clone()), Some('\u{E8C8}')));
            if let Some(parent) = p(path).parent().map(|x| x.to_string_lossy().to_string()).filter(|x| x.len() > 3) {
                copy.push(Entry::separator());
                copy.push(item(&parent, Cmd::Copy(parent.clone()), None));
            }
            let name = p(path).file_name().map(|x| x.to_string_lossy().to_string()).unwrap_or_default();
            if !name.is_empty() {
                copy.push(Entry::separator());
                copy.push(item(&name, Cmd::Copy(name.clone()), None));
            }
            if ctx.kind == Kind::File {
                let stem = p(path).file_stem().map(|x| x.to_string_lossy().to_string()).unwrap_or_default();
                if !stem.is_empty() && stem != name {
                    copy.push(item(&stem, Cmd::Copy(stem.clone()), None));
                }
                if let Some(ext) = p(path).extension().map(|x| x.to_string_lossy().to_string()) {
                    copy.push(item(&ext, Cmd::Copy(ext.clone()), None));
                }
            }
        }
        v.push(folder("Copy path", '\u{E8C8}', copy));
    }
    v.push(folder(
        "Select",
        '\u{E8B3}',
        vec![item("All", Cmd::Select(0), Some('\u{E8B3}')), item("Invert", Cmd::Select(1), Some('\u{E8AB}')), item("None", Cmd::Select(2), Some('\u{E894}'))],
    ));
    v.push(Entry::separator());
    let hidden_shown = explorer_flag("Hidden") == Some(1);
    let ext_shown = explorer_flag("HideFileExt") == Some(0);
    v.push(folder(
        "Show / Hide",
        '\u{E7B3}',
        vec![check(item("Hidden files", Cmd::ShowHidden, None), hidden_shown), check(item("File name extensions", Cmd::ShowExtensions, None), ext_shown)],
    ));
    if matches!(ctx.kind, Kind::File | Kind::Folder) && ctx.paths.len() == 1 {
        let a = attributes(&ctx.paths[0]);
        let attr = |name: &str, bit: u32| check(item(name, Cmd::Attribute(bit), None), a & bit != 0);
        v.push(folder(
            "Attributes",
            '\u{E946}',
            vec![attr("Hidden", FILE_ATTRIBUTE_HIDDEN), attr("Read-only", FILE_ATTRIBUTE_READONLY), attr("Archive", FILE_ATTRIBUTE_ARCHIVE)],
        ));
    }
    if !ctx.kind.selection() {
        v.push(Entry::separator());
        v.push(item("New folder (date and time)", Cmd::NewFolder, Some('\u{E8F4}')));
        v.push(folder(
            "New file",
            '\u{E8A5}',
            vec![item("Text", Cmd::NewFile("txt"), None), item("Markdown", Cmd::NewFile("md"), None), item("JSON", Cmd::NewFile("json"), None)],
        ));
    }
    if ctx.kind != Kind::Desktop {
        v.push(Entry::separator());
        v.push(item("Folder options", Cmd::Run("control.exe".into(), "folders".into()), Some('\u{E713}')));
    }
    v
}

fn go_to() -> Vec<Entry> {
    let env = |k: &str| std::env::var(k).unwrap_or_default();
    let (profile, windir) = (env("USERPROFILE"), env("WINDIR"));
    let open = |name: &str, target: String| item(name, Cmd::Open(target), None);
    let folders = vec![
        open("Windows", windir.clone()),
        open("System", format!(r"{windir}\System32")),
        open("Program Files", env("ProgramFiles")),
        open("Program Files (x86)", env("ProgramFiles(x86)")),
        open("ProgramData", env("ProgramData")),
        open("Applications", "shell:AppsFolder".into()),
        open("Users", std::path::Path::new(&profile).parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_default()),
        Entry::separator(),
        open("Desktop", "shell:Desktop".into()),
        open("Downloads", "shell:Downloads".into()),
        open("Pictures", "shell:My Pictures".into()),
        open("Documents", "shell:Personal".into()),
        open("Start menu", "shell:Start Menu".into()),
        open("Profile", profile.clone()),
        open("AppData", env("APPDATA")),
        open("Temp", env("TEMP")),
    ];
    let settings = |pairs: &[(&str, &str)]| pairs.iter().map(|(n, u)| open(n, (*u).into())).collect::<Vec<_>>();
    vec![
        folder("Folder", '\u{E8B7}', folders),
        item("Control Panel", Cmd::Open("shell:::{5399E694-6CE5-4D6C-8FCE-1D8870FDCBA0}".into()), Some('\u{E713}')),
        item("All Control Panel items", Cmd::Open("shell:::{ED7BA470-8E54-465E-825C-99712043E01C}".into()), Some('\u{E713}')),
        item("Run", Cmd::Open("shell:::{2559a1f3-21d7-11d4-bdaf-00c04f60b9f0}".into()), Some('\u{E768}')),
        Entry::separator(),
        folder(
            "Settings",
            '\u{E713}',
            vec![
                open("System", "ms-settings:".into()),
                open("About", "ms-settings:about".into()),
                open("Windows Update", "ms-settings:windowsupdate".into()),
                open("Windows Security", "ms-settings:windowsdefender".into()),
                Entry::separator(),
                folder("Apps", '\u{E71D}', settings(&[("Installed apps", "ms-settings:appsfeatures"), ("Default apps", "ms-settings:defaultapps"), ("Startup", "ms-settings:startupapps")])),
                folder(
                    "Personalization",
                    '\u{E771}',
                    settings(&[
                        ("Personalization", "ms-settings:personalization"),
                        ("Background", "ms-settings:personalization-background"),
                        ("Colors", "ms-settings:colors"),
                        ("Themes", "ms-settings:themes"),
                        ("Taskbar", "ms-settings:taskbar"),
                    ]),
                ),
                folder("Network", '\u{E968}', settings(&[("Status", "ms-settings:network-status"), ("Ethernet", "ms-settings:network-ethernet")])),
            ],
        ),
    ]
}

/// Runs one of Kasumi's own entries.
pub fn run(cmd: &Cmd, ctx: &Ctx, t: &Target) {
    let shell_open = |file: &str, args: &str, dir: &str| unsafe {
        let (f, a, d) = (HSTRING::from(file), HSTRING::from(args), HSTRING::from(dir));
        ShellExecuteW(None, windows::core::w!("open"), &f, if args.is_empty() { PCWSTR::null() } else { PCWSTR(a.as_ptr()) }, &d, SW_SHOWNORMAL);
    };
    match cmd {
        Cmd::Run(exe, args) => shell_open(exe, args, &ctx.here),
        Cmd::Open(target) => shell_open(target, "", &ctx.here),
        Cmd::Copy(text) => crate::win::set_clipboard_text(text),
        Cmd::Select(how) => shell::select(t, *how),
        Cmd::ShowHidden | Cmd::ShowExtensions => {
            let (name, value) = match cmd {
                Cmd::ShowHidden => ("Hidden", if explorer_flag("Hidden") == Some(1) { 2 } else { 1 }),
                _ => ("HideFileExt", if explorer_flag("HideFileExt") == Some(0) { 1 } else { 0 }),
            };
            crate::win::reg_set_dword(r"Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced", name, value);
            crate::win::broadcast_setting("ShellState");
            shell::run_view(t, ViewCmd::Refresh);
        }
        Cmd::Attribute(bit) => {
            for p in &ctx.paths {
                let a = attributes(p);
                let w = HSTRING::from(p.as_str());
                let next = if a & bit != 0 { a & !bit } else { a | bit };
                unsafe {
                    let _ = windows::Win32::Storage::FileSystem::SetFileAttributesW(PCWSTR(w.as_ptr()), windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES(next));
                }
            }
        }
        Cmd::NewFolder => {
            let _ = std::fs::create_dir(std::path::Path::new(&ctx.dir).join(stamp()));
        }
        Cmd::NewFile(ext) => {
            let _ = std::fs::write(std::path::Path::new(&ctx.dir).join(format!("{}.{ext}", stamp())), "");
        }
    }
}

/// A name from the date and time (2026-10-22 19-45-03).
fn stamp() -> String {
    let t = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    format!("{:04}-{:02}-{:02} {:02}-{:02}-{:02}", t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond)
}
