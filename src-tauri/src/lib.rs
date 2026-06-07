#[macro_export]
macro_rules! debug_log {
    ($($arg:tt)*) => {
        #[cfg(debug_assertions)]
        println!($($arg)*);
    };
}

mod archive;
mod directory;
mod media;
mod settings;
mod upscale;

use archive::{
    cleanup_temp_dir, clear_session, delete_single_temp_dir, extract_archive,
    extract_archive_from_path, extract_archive_with_nested, extract_nested_streaming, extract_rar,
    extract_rar_from_path, extract_rar_with_nested, extract_to_temp, handle_file_drop, ExtractState,
};
use directory::{list_directory, scan_directory};
use settings::{load_settings, save_settings};
use std::{fs, path::PathBuf};
use tauri::{command, Manager, Window};
use upscale::{download_ncnn_vulkan, get_active_ep, upscale_image, UpscaleSessionCache};

#[command]
fn toggle_fullscreen(window: Window) -> Result<bool, String> {
    let is_fullscreen = window.is_fullscreen().map_err(|err| format!("Failed to read fullscreen state: {err}"))?;
    let next = !is_fullscreen;
    window.set_fullscreen(next).map_err(|err| format!("Failed to set fullscreen: {err}"))?;
    Ok(next)
}

// ─── 起動時 CUDA パス設定 ────────────────────────────────────────────────────

#[cfg(target_os = "windows")]
fn setup_cuda_paths() {
    let mut extra: Vec<PathBuf> = Vec::new();
    if let Ok(cuda_path) = std::env::var("CUDA_PATH") {
        extra.push(PathBuf::from(cuda_path).join("bin"));
    }
    if let Ok(cudnn_path) = std::env::var("CUDNN_PATH") {
        extra.push(PathBuf::from(cudnn_path).join("bin"));
    } else {
        let base = PathBuf::from(
            std::env::var("ProgramFiles").unwrap_or_else(|_| "C:\\Program Files".into()),
        ).join("NVIDIA").join("CUDNN");
        if let Ok(vers) = fs::read_dir(&base) {
            for ver in vers.flatten() {
                if let Ok(bins) = fs::read_dir(ver.path().join("bin")) {
                    for cuda_ver in bins.flatten() {
                        let x64 = cuda_ver.path().join("x64");
                        if x64.is_dir() { extra.push(x64); }
                    }
                }
            }
        }
    }
    let extra: Vec<_> = extra.into_iter().filter(|p| p.is_dir())
        .map(|p| p.to_string_lossy().into_owned()).collect();
    if !extra.is_empty() {
        let current = std::env::var("PATH").unwrap_or_default();
        std::env::set_var("PATH", format!("{};{current}", extra.join(";")));
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run(context: tauri::Context<tauri::Wry>) {
    #[cfg(target_os = "windows")]
    setup_cuda_paths();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(ExtractState::default())
        .manage(UpscaleSessionCache::default())
        .invoke_handler(tauri::generate_handler![
            extract_archive,
            extract_archive_from_path,
            extract_rar_from_path,
            extract_archive_with_nested,
            extract_rar_with_nested,
            extract_nested_streaming,
            extract_rar,
            handle_file_drop,
            scan_directory,
            extract_to_temp,
            clear_session,
            delete_single_temp_dir,
            list_directory,
            toggle_fullscreen,
            load_settings,
            save_settings,
            upscale_image,
            download_ncnn_vulkan,
            get_active_ep
        ])
        .build(context)
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            if let tauri::RunEvent::Exit = event {
                debug_log!("[exit] cleaning up session dirs");
                if let Some(state) = app_handle.try_state::<ExtractState>() {
                    if let Ok(mut guard) = state.session_dirs.lock() {
                        for dir in guard.drain(..) {
                            debug_log!("[exit] deleting: {}", dir.display());
                            cleanup_temp_dir(&dir);
                        }
                    }
                }
            }
        });
}
