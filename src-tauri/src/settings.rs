use std::{fs, path::PathBuf};
use tauri::{command, AppHandle, Manager};

fn settings_path(app: &AppHandle) -> Result<PathBuf, String> {
    let exe_path = std::env::current_exe().map_err(|err| format!("Failed to resolve current exe: {err}"))?;
    if let Some(dir) = exe_path.parent() { return Ok(dir.join("settings.ini")); }
    let dir = app.path().executable_dir().map_err(|err| format!("Failed to resolve executable directory: {err}"))?;
    Ok(dir.join("settings.ini"))
}

#[command]
pub fn load_settings(app: AppHandle) -> Result<Option<String>, String> {
    let path = settings_path(&app)?;
    debug_log!("load_settings: path={}", path.display());
    if !path.exists() { return Ok(None); }
    let contents = fs::read_to_string(&path).map_err(|err| format!("Failed to read settings: {err}"))?;
    debug_log!("load_settings: loaded {} bytes", contents.len());
    Ok(Some(contents))
}

#[command]
pub fn save_settings(app: AppHandle, contents: String) -> Result<(), String> {
    let path = settings_path(&app)?;
    debug_log!("save_settings: path={} bytes={}", path.display(), contents.len());
    fs::write(path, contents).map_err(|err| format!("Failed to save settings: {err}"))?;
    debug_log!("save_settings: done");
    Ok(())
}
