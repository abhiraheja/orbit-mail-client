//! Tauri wiring: module declarations, state setup, command registration.

mod ai;
mod commands;
mod db;
mod error;
mod events;
mod graph;
mod loops;
mod models;
mod secrets;
mod state;
mod sync;

use tauri::Manager;

use state::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            // Store the DB in the platform app-data dir so it survives upgrades
            // and stays per-user. Created on first launch.
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            let db_path = dir.join("orbit.sqlite3");
            let state = AppState::new(db_path)
                .map_err(|e| format!("failed to initialize database: {e}"))?;
            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::ping,
            commands::emit_test_event,
            commands::add_account,
            commands::list_accounts,
            commands::remove_account,
            commands::sync_account,
            commands::list_loops,
            commands::snooze_loop,
            commands::dismiss_loop,
            commands::get_thread,
            commands::list_contacts,
            commands::draft_reply,
            commands::get_ai_audit_log,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
