// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let context = tauri::generate_context!("tauri.conf.json");
    c_userssatakeappdatalocaltemptauri_svelte_app_lib::run(context)
}
