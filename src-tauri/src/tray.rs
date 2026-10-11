use crate::{autostart, config, engine::ReloadSender};
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::Manager;

pub fn setup(app: &tauri::App) -> tauri::Result<()> {
    let settings = MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?;
    let open = MenuItem::with_id(app, "open", "Open config file", true, None::<&str>)?;
    let folder = MenuItem::with_id(app, "folder", "Open config folder", true, None::<&str>)?;
    let reload = MenuItem::with_id(app, "reload", "Reload config", true, None::<&str>)?;
    let startup = CheckMenuItem::with_id(app, "autostart", "Start with Windows", true, autostart::is_enabled(), None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Kasumi", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&settings, &sep, &open, &folder, &reload, &sep, &startup, &sep, &quit])?;
    let startup_item = startup.clone();

    let mut tray = TrayIconBuilder::with_id("main")
        .tooltip("Kasumi")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(move |app, event| match event.id().as_ref() {
            "settings" => crate::settings::open(app),
            "open" => {
                let _ = std::process::Command::new("notepad.exe").arg(config::path()).spawn();
            }
            "folder" => {
                let _ = std::process::Command::new("explorer.exe").arg(config::dir()).spawn();
            }
            "reload" => {
                let _ = app.state::<ReloadSender>().0.send(());
            }
            "autostart" => {
                // The check mark has already toggled; make the registry match it.
                let want = startup_item.is_checked().unwrap_or(false);
                if let Err(e) = autostart::set(want) {
                    eprintln!("[kasumi] autostart: {e}");
                    let _ = startup_item.set_checked(autostart::is_enabled());
                }
            }
            "quit" => app.exit(0),
            _ => {}
        });
    // The white mark: the app icon's black square would be lost on a dark taskbar.
    match tauri::image::Image::from_bytes(include_bytes!("../icons/tray.png")) {
        Ok(icon) => tray = tray.icon(icon),
        Err(_) => {
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
        }
    }
    tray.build(app)?;
    Ok(())
}
