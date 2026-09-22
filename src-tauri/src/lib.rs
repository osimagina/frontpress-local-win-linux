mod commands;
mod frontpress;
mod keychain;
mod net;
mod paths;
mod php;
mod server;
mod siteops;
mod store;
mod util;

use commands::AppState;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{Emitter, Manager};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::load())
        .menu(|handle| {
            // Cross-platform menu (Windows + Linux): File > Quit and
            // Help > Check for Updates… (emits "menu:check-updates" which the
            // frontend updater hook listens for). Edit keeps copy/paste
            // shortcuts working in text fields.
            let check =
                MenuItem::with_id(handle, "check-updates", "Check for Updates…", true, None::<&str>)?;
            let file_menu = Submenu::with_items(
                handle,
                "File",
                true,
                &[&PredefinedMenuItem::quit(handle, None)?],
            )?;
            let edit_menu = Submenu::with_items(
                handle,
                "Edit",
                true,
                &[
                    &PredefinedMenuItem::undo(handle, None)?,
                    &PredefinedMenuItem::redo(handle, None)?,
                    &PredefinedMenuItem::separator(handle)?,
                    &PredefinedMenuItem::cut(handle, None)?,
                    &PredefinedMenuItem::copy(handle, None)?,
                    &PredefinedMenuItem::paste(handle, None)?,
                    &PredefinedMenuItem::select_all(handle, None)?,
                ],
            )?;
            let help_menu = Submenu::with_items(handle, "Help", true, &[&check])?;
            Menu::with_items(handle, &[&file_menu, &edit_menu, &help_menu])
        })
        .on_menu_event(|app, event| {
            if event.id() == "check-updates" {
                // The frontend's updater hook listens for this and runs the
                // same check that powers the in-app banner.
                let _ = app.emit("menu:check-updates", ());
            }
        })
        .setup(|app| {
            // Stamp the running version to disk so a self-update can be
            // verified out-of-band (the file flips to the new version after
            // the updater relaunches the app).
            if let Ok(dir) = crate::paths::app_data_dir() {
                let v = app.package_info().version.to_string();
                let _ = std::fs::write(dir.join("last-run-version.txt"), v);
            }
            // Discover any sites already in the configured folder (e.g. a
            // Dropbox folder synced from another machine).
            if let Some(state) = app.try_state::<AppState>() {
                let _ = commands::discover_sites(state.inner());
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_status,
            commands::available_php,
            commands::install_php,
            commands::set_global_php,
            commands::create_site,
            commands::start_site,
            commands::stop_site,
            commands::stop_all_sites,
            commands::duplicate_site,
            commands::import_site,
            commands::backup_site,
            commands::restore_into_site,
            commands::delete_site,
            commands::open_preview,
            commands::auto_login,
            commands::reveal_in_finder,
            commands::open_in_editor,
            commands::list_editors,
            commands::set_editor,
            commands::set_sites_dir,
            commands::rescan_sites,
            commands::get_settings,
            commands::selftest_update,
        ])
        .build(tauri::generate_context!())
        .expect("error while building FrontPress Local")
        .run(|app_handle, event| {
            // Make sure no orphan `php -S` processes survive the app.
            if let tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit = event {
                if let Some(state) = app_handle.try_state::<AppState>() {
                    state.servers.stop_all();
                }
            }
        });
}
