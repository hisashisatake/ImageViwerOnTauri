mod rar;
mod zip;

pub use rar::{extract_rar, extract_rar_from_path, extract_rar_with_nested};
pub use zip::{extract_archive, extract_archive_from_path, extract_archive_with_nested};

use crate::media::{
    entry_name, is_supported_archive, is_supported_image, is_supported_pdf, is_supported_rar,
    sanitize_name,
};
use rar::{count_rar_entries, extract_rar_file, extract_rar_to_folder, stream_rar};
use serde::Serialize;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::{command, ipc::Channel, State};
use zip::{extract_archive_bytes, extract_zip_to_folder, stream_zip};

#[derive(Default)]
pub struct ExtractState {
    pub session_dirs: Mutex<Vec<PathBuf>>,
}

impl Drop for ExtractState {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.session_dirs.lock() {
            for dir in guard.drain(..) {
                cleanup_temp_dir(&dir);
            }
        }
    }
}

#[derive(Serialize, Clone)]
pub struct ExtractProgress {
    current: usize,
    total: usize,
}

#[derive(Serialize, Clone)]
pub struct ExtractedFile {
    path: String,
    name: String,
    size: u64,
}

#[derive(Serialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Phase1Event {
    Progress { current: usize, total: usize },
    Done { images: Vec<ExtractedFile>, nested_archives: Vec<String> },
    Error { message: String },
}

pub fn cleanup_temp_dir(dir: &Path) {
    let _ = fs::remove_dir_all(dir);
    if let Some(parent) = dir.parent() {
        let _ = fs::remove_dir(parent); // 空なら削除、空でなければ無視
    }
}

fn clear_session_dirs(state: &State<ExtractState>) -> Result<(), String> {
    let mut guard = state
        .session_dirs
        .lock()
        .map_err(|_| "Failed to lock state".to_string())?;
    debug_log!("[clear_session] dirs to delete: {}", guard.len());
    for dir in guard.drain(..) {
        debug_log!("[clear_session] deleting: {}", dir.display());
        let result = fs::remove_dir_all(&dir);
        debug_log!("[clear_session] result: {:?}", result);
    }
    Ok(())
}

fn create_extract_dir(state: &State<ExtractState>, archive_name: &str) -> Result<PathBuf, String> {
    let temp_root = std::env::temp_dir().join("viewer-on-tauri");
    fs::create_dir_all(&temp_root)
        .map_err(|err| format!("Failed to create temp root: {err}"))?;

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|err| format!("Failed to read system time: {err}"))?
        .as_millis();
    let safe_name = sanitize_name(archive_name);
    let extract_dir = temp_root.join(format!("{timestamp}-{safe_name}"));
    fs::create_dir_all(&extract_dir)
        .map_err(|err| format!("Failed to create temp dir: {err}"))?;

    let mut last_dir_guard = state
        .session_dirs
        .lock()
        .map_err(|_| "Failed to lock state".to_string())?;
    last_dir_guard.push(extract_dir.clone());
    Ok(extract_dir)
}

// ─── 共通ヘルパー（zip/rar 双方のサブモジュールから利用）───────────────────────

fn make_outpath(rel_path: &Path, extract_dir: &Path) -> Result<PathBuf, String> {
    let out = extract_dir.join(rel_path);
    if let Some(p) = out.parent() {
        fs::create_dir_all(p).map_err(|e| format!("mkdir: {e}"))?;
    }
    Ok(out)
}

fn push_image(out: &Path, rel: &Path, size: u64, images: &mut Vec<ExtractedFile>) {
    images.push(ExtractedFile {
        path: out.to_string_lossy().to_string(),
        name: entry_name(rel),
        size,
    });
}

fn send_image(out: &Path, rel: &Path, size: u64, ch: &Channel<ExtractedFile>) -> Result<(), String> {
    ch.send(ExtractedFile {
        path: out.to_string_lossy().to_string(),
        name: entry_name(rel),
        size,
    }).map_err(|e| format!("Channel: {e}"))
}

fn sort_by_path(files: &mut Vec<ExtractedFile>) {
    files.sort_by(|a, b| a.path.to_lowercase().cmp(&b.path.to_lowercase()));
}

// ─── コマンド（zip/rar 両方を扱う / セッション管理）────────────────────────────

#[command]
pub fn extract_nested_streaming(
    archive_path: String,
    channel: Channel<ExtractedFile>,
) -> Result<(), String> {
    let path = PathBuf::from(&archive_path);
    let stem = path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "nested".to_string());
    let out_dir = path.parent().unwrap_or(&path).join(&stem);
    fs::create_dir_all(&out_dir).map_err(|e| format!("Create dir: {e}"))?;
    if is_supported_archive(&path) {
        let bytes = fs::read(&path).map_err(|e| format!("Read: {e}"))?;
        stream_zip(bytes, &out_dir, &channel)?;
    } else if is_supported_rar(&path) {
        stream_rar(&path, &out_dir, &channel)?;
    }
    let _ = fs::remove_file(&path);
    Ok(())
}

#[command]
pub fn handle_file_drop(
    state: State<ExtractState>,
    paths: Vec<String>,
) -> Result<Vec<ExtractedFile>, String> {
    // セッション内のクリアは行わない（clear_session コマンドで一括管理）
    let mut extracted = Vec::new();
    let mut archive_dir: Option<PathBuf> = None;

    for path_str in paths {
        let path = PathBuf::from(&path_str);
        if !path.is_file() { continue; }

        if is_supported_image(&path) {
            let metadata = fs::metadata(&path).map_err(|err| format!("Failed to read metadata: {err}"))?;
            let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "image".to_string());
            extracted.push(ExtractedFile { path: path.to_string_lossy().to_string(), name, size: metadata.len() });
            continue;
        }
        if is_supported_archive(&path) {
            let bytes = fs::read(&path).map_err(|err| format!("Failed to read archive: {err}"))?;
            let dir = if let Some(existing) = &archive_dir { existing.clone() } else {
                let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "archive".to_string());
                let new_dir = create_extract_dir(&state, &name)?;
                archive_dir = Some(new_dir.clone());
                new_dir
            };
            let mut items = extract_archive_bytes(bytes, &dir, true)?;
            extracted.append(&mut items);
        }
        if is_supported_rar(&path) {
            let dir = if let Some(existing) = &archive_dir { existing.clone() } else {
                let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "archive".to_string());
                let new_dir = create_extract_dir(&state, &name)?;
                archive_dir = Some(new_dir.clone());
                new_dir
            };
            let mut items = extract_rar_file(&path, &dir, true)?;
            extracted.append(&mut items);
        }
        if is_supported_pdf(&path) {
            let metadata = fs::metadata(&path).map_err(|err| format!("Failed to read metadata: {err}"))?;
            let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "document".to_string());
            extracted.push(ExtractedFile { path: path.to_string_lossy().to_string(), name, size: metadata.len() });
        }
    }
    sort_by_path(&mut extracted);
    Ok(extracted)
}

// アーカイブを temp に展開してフォルダパスを返す（ZIP = フォルダと同義）
// 呼び出し側は scan_directory でフォルダとして扱う
#[command]
pub async fn extract_to_temp(
    state: State<'_, ExtractState>,
    archive_path: String,
    channel: Channel<ExtractProgress>,
) -> Result<String, String> {
    let archive = PathBuf::from(&archive_path);
    let stem = archive.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "archive".to_string());
    let file_name = archive.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "archive".to_string());

    let temp_root = std::env::temp_dir().join("viewer-on-tauri");
    let ts = SystemTime::now().duration_since(UNIX_EPOCH)
        .map_err(|e| format!("Time: {e}"))?.as_millis();
    let session_dir = temp_root.join(format!("{ts}-{}", sanitize_name(&file_name)));
    let extract_dir = session_dir.join(&stem);
    fs::create_dir_all(&extract_dir).map_err(|e| format!("mkdir: {e}"))?;

    {
        let mut guard = state.session_dirs.lock()
            .map_err(|_| "Lock error".to_string())?;
        debug_log!("[extract_to_temp] registering: {}", session_dir.display());
        guard.push(session_dir.clone());
    }

    let is_rar = is_supported_rar(&archive);
    let folder_path = extract_dir.to_string_lossy().to_string();
    tauri::async_runtime::spawn_blocking(move || {
        if is_rar {
            let total = count_rar_entries(&archive);
            extract_rar_to_folder(&archive, &extract_dir, |c, _| {
                let _ = channel.send(ExtractProgress { current: c, total });
            })
        } else {
            fs::read(&archive)
                .map_err(|e| format!("Read: {e}"))
                .and_then(|bytes| extract_zip_to_folder(bytes, &extract_dir, |c, t| {
                    let _ = channel.send(ExtractProgress { current: c, total: t });
                }))
        }
    }).await.map_err(|e| format!("Task: {e}"))??;

    Ok(folder_path)
}

#[command]
pub fn clear_session(state: State<ExtractState>) -> Result<(), String> {
    clear_session_dirs(&state)
}

// セッション切り替え時に特定の一時フォルダを削除
#[command]
pub fn delete_single_temp_dir(state: State<ExtractState>, path: String) -> Result<(), String> {
    let dir = PathBuf::from(&path);
    if let Ok(mut guard) = state.session_dirs.lock() {
        guard.retain(|d| d != &dir);
    }
    cleanup_temp_dir(&dir);
    Ok(())
}
