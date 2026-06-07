use crate::media::{entry_name, is_supported_archive, is_supported_image, is_supported_pdf, is_supported_rar};
use serde::Serialize;
use std::{fs, path::PathBuf};
use tauri::command;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderEntry {
    path: String,
    name: String,
    entry_type: String, // "image", "zip", "rar", "pdf"
    size: u64,
}

// フォルダをスキャンして画像・アーカイブのソート済みリストを返す
#[command]
pub fn scan_directory(path: String) -> Result<Vec<FolderEntry>, String> {
    let dir = PathBuf::from(&path);
    if !dir.is_dir() {
        return Err(format!("Not a directory: {path}"));
    }
    let mut entries: Vec<FolderEntry> = fs::read_dir(&dir)
        .map_err(|e| format!("Read dir: {e}"))?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_file())
        .filter_map(|e| {
            let p = e.path();
            let et = if is_supported_image(&p) { "image" }
                else if is_supported_archive(&p) { "zip" }
                else if is_supported_rar(&p) { "rar" }
                else if is_supported_pdf(&p) { "pdf" }
                else { return None; };
            Some(FolderEntry {
                path: p.to_string_lossy().to_string(),
                name: entry_name(&p),
                entry_type: et.to_string(),
                size: e.metadata().map(|m| m.len()).unwrap_or(0),
            })
        })
        .collect();
    entries.sort_by(|a, b| a.path.to_lowercase().cmp(&b.path.to_lowercase()));
    Ok(entries)
}

#[command]
pub fn list_directory(path: String) -> Result<Vec<String>, String> {
    let dir = PathBuf::from(&path);
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut files: Vec<String> = fs::read_dir(&dir)
        .map_err(|e| format!("Read dir: {e}"))?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_file())
        .map(|e| e.path().to_string_lossy().to_string())
        .collect();
    files.sort();
    Ok(files)
}
