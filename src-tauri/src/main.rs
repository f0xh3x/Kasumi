#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod actions;
mod autostart;
mod config;
mod config_edit;
mod engine;
mod instance;
mod logfile;
mod osd;
mod presets;
mod providers;
mod settings;
mod tray;
mod win;

fn main() {
    logfile::init();
    instance::claim();
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![engine::get_layout, engine::set_regions, engine::fe_log, engine::set_content_height, engine::set_notch_widths, actions::run_action, providers::get_data, providers::media::media_control, providers::systray::tray_click, osd::switch_keyboard_layout, config_edit::config_read, config_edit::config_write, config_edit::config_reset, autostart::autostart_get, autostart::autostart_set, settings::open_settings, settings::app_version, settings::monitors_list, settings::lhm_sensors, settings::system_time_format, config_edit::config_write_many, presets::presets_list, presets::preset_save, presets::preset_apply, presets::preset_rename, presets::preset_delete, providers::tasks::task_activate, providers::tasks::task_close, providers::tasks::task_launch])
        .setup(|app| {
            engine::start(app.handle());
            osd::start(app.handle());
            autostart::refresh();
            tray::setup(app)?;
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to build StatusNotch")
        .run(|_app, event| {
            // Overlays come and go with monitors; only exit on an explicit app.exit().
            if let tauri::RunEvent::ExitRequested { code: None, api, .. } = event {
                api.prevent_exit();
            }
        });
}
