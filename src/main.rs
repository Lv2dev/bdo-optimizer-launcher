#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod backend;
mod tauri_commands;
mod tauri_lifecycle;

fn main() {
    let _log_guard = backend::logging::init();
    tracing::info!(args = ?std::env::args().collect::<Vec<_>>(), "main start");

    if !backend::singleton::acquire_or_focus_existing() {
        tracing::info!("singleton: existing instance found, exiting");
        std::process::exit(0);
    }

    tauri::Builder::default()
        .setup(tauri_lifecycle::setup)
        .invoke_handler(tauri::generate_handler![
            tauri_commands::get_app_state,
            tauri_commands::get_settings,
            tauri_commands::get_monitor_snapshot,
            tauri_commands::stop_monitor_session,
            tauri_commands::set_setting,
            tauri_commands::open_log_folder,
            tauri_commands::check_for_updates,
            tauri_commands::check_update_alert,
            tauri_commands::open_update_release,
            tauri_commands::open_repository,
            tauri_commands::refresh_game_status,
            tauri_commands::launch_game,
            tauri_commands::apply_mode,
            tauri_commands::list_schedule_rules,
            tauri_commands::add_schedule_rule,
            tauri_commands::delete_schedule_rule,
            tauri_commands::toggle_schedule_rule,
            tauri_commands::get_shutdown_state,
            tauri_commands::register_shutdown,
            tauri_commands::cancel_shutdown,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
