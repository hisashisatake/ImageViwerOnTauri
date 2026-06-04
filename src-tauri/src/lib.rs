use ort::{session::Session, value::Tensor as OrtTensor};

macro_rules! debug_log {
    ($($arg:tt)*) => {
        #[cfg(debug_assertions)]
        println!($($arg)*);
    };
}
use serde::Serialize;
use std::{
    collections::HashMap,
    fs,
    io::{Cursor, Read},
    path::{Path, PathBuf},
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::{command, AppHandle, Manager, State, Window};
use unrar::Archive;
use zip::ZipArchive;

#[derive(Default)]
struct ExtractState {
    session_dirs: Mutex<Vec<PathBuf>>,
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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FolderEntry {
    path: String,
    name: String,
    entry_type: String, // "image", "zip", "rar", "pdf"
    size: u64,
}

#[derive(Default)]
struct UpscaleSessionCache {
    sessions: Mutex<HashMap<(u32, String), Session>>,
}

#[derive(Serialize, Clone)]
struct ExtractedFile {
    path: String,
    name: String,
    size: u64,
}

#[derive(Serialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Phase1Event {
    Progress { current: usize, total: usize },
    Done { images: Vec<ExtractedFile>, nested_archives: Vec<String> },
    Error { message: String },
}

fn sanitize_name(name: &str) -> String {
    let mut sanitized = String::with_capacity(name.len());
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            sanitized.push(ch);
        } else {
            sanitized.push('_');
        }
    }
    if sanitized.is_empty() {
        "archive".to_string()
    } else {
        sanitized
    }
}

fn is_supported_image(path: &Path) -> bool {
    let ext = if let Some(ext) = path.extension().and_then(|ext| ext.to_str()) {
        ext.to_ascii_lowercase()
    } else {
        return false;
    };
    return matches!(
        ext.as_str(),
        "png" | "jpg" | "jpeg" | "webp" | "gif" | "svg" | "bmp" | "avif"
    );
}

fn is_supported_archive(path: &Path) -> bool {
    let ext = if let Some(ext) = path.extension().and_then(|ext| ext.to_str()) {
        ext.to_ascii_lowercase()
    } else {
        return false;
    };
    return matches!(ext.as_str(), "zip" | "cbz");
}

fn is_supported_rar(path: &Path) -> bool {
    let ext = if let Some(ext) = path.extension().and_then(|ext| ext.to_str()) {
        ext.to_ascii_lowercase()
    } else {
        return false;
    };
    return matches!(ext.as_str(), "rar");
}

fn is_supported_pdf(path: &Path) -> bool {
    let ext = if let Some(ext) = path.extension().and_then(|ext| ext.to_str()) {
        ext.to_ascii_lowercase()
    } else {
        return false;
    };
    return matches!(ext.as_str(), "pdf");
}

fn safe_relative_path(path: &Path) -> Option<PathBuf> {
    let mut clean = PathBuf::new();
    for component in path.components() {
        if let std::path::Component::Normal(segment) = component {
            clean.push(segment);
        }
    }
    if clean.as_os_str().is_empty() {
        None
    } else {
        Some(clean)
    }
}

fn cleanup_temp_dir(dir: &Path) {
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

// ─── 共通ヘルパー ───────────────────────────────────────────────────────────

fn entry_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "image".to_string())
}

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

fn send_image(out: &Path, rel: &Path, size: u64, ch: &tauri::ipc::Channel<ExtractedFile>) -> Result<(), String> {
    ch.send(ExtractedFile {
        path: out.to_string_lossy().to_string(),
        name: entry_name(rel),
        size,
    }).map_err(|e| format!("Channel: {e}"))
}

fn sort_by_path(files: &mut Vec<ExtractedFile>) {
    files.sort_by(|a, b| a.path.to_lowercase().cmp(&b.path.to_lowercase()));
}

fn count_rar_entries(path: &Path) -> usize {
    let Ok(mut archive) = Archive::new(path).open_for_processing() else { return 0; };
    let mut count = 0;
    loop {
        let Ok(Some(h)) = archive.read_header() else { break; };
        if h.entry().is_file() { count += 1; }
        let Ok(next) = h.skip() else { break; };
        archive = next;
    }
    count
}

fn run_archive_outer_zip(bytes: Vec<u8>, extract_dir: &Path, ch: &tauri::ipc::Channel<Phase1Event>) {
    let mut archive = match ZipArchive::new(Cursor::new(bytes)) {
        Ok(a) => a,
        Err(e) => { let _ = ch.send(Phase1Event::Error { message: format!("Invalid archive: {e}") }); return; }
    };
    let total = archive.len();
    let mut images = Vec::new();
    let mut nested_archives = Vec::new();
    let mut last_sent = std::time::Instant::now();

    for i in 0..total {
        let mut f = match archive.by_index(i) {
            Ok(f) => f, Err(_) => continue,
        };
        if f.is_dir() { continue; }
        let Some(enc) = f.enclosed_name().map(|p| p.to_owned()) else { continue; };

        if is_supported_image(&enc) {
            if let Ok(out) = make_outpath(&enc, extract_dir) {
                if let Ok(mut of) = fs::File::create(&out) {
                    let _ = std::io::copy(&mut f, &mut of);
                    push_image(&out, &enc, f.size(), &mut images);
                }
            }
        } else if is_supported_archive(&enc) || is_supported_rar(&enc) {
            let out = extract_dir.join(entry_name(&enc));
            if let Ok(mut of) = fs::File::create(&out) {
                let _ = std::io::copy(&mut f, &mut of);
                nested_archives.push(out.to_string_lossy().to_string());
            }
        }
        if last_sent.elapsed().as_millis() >= 100 || i + 1 == total {
            let _ = ch.send(Phase1Event::Progress { current: i + 1, total });
            last_sent = std::time::Instant::now();
        }
    }
    sort_by_path(&mut images);
    let _ = ch.send(Phase1Event::Done { images, nested_archives });
}

fn run_archive_outer_rar(path: PathBuf, extract_dir: &Path, ch: &tauri::ipc::Channel<Phase1Event>) {
    let total = count_rar_entries(&path);
    let mut archive = match Archive::new(&path).open_for_processing() {
        Ok(a) => a,
        Err(e) => { let _ = ch.send(Phase1Event::Error { message: format!("Open rar: {e}") }); return; }
    };
    let mut images = Vec::new();
    let mut nested_archives = Vec::new();
    let mut current = 0usize;
    let mut last_sent = std::time::Instant::now();

    loop {
        let Ok(Some(h)) = archive.read_header() else { break; };
        if !h.entry().is_file() {
            let Ok(next) = h.skip() else { break; };
            archive = next;
            continue;
        }
        let fname = h.entry().filename.clone();
        let size = h.entry().unpacked_size;
        let Some(rel) = safe_relative_path(&fname) else {
            let Ok(next) = h.skip() else { break; };
            archive = next;
            continue;
        };

        if is_supported_image(&rel) {
            if let Ok(out) = make_outpath(&rel, extract_dir) {
                if let Ok(next) = h.extract_to(&out) {
                    push_image(&out, &rel, size, &mut images);
                    archive = next;
                } else { break; }
            } else { let Ok(next) = h.skip() else { break; }; archive = next; }
        } else if is_supported_archive(&rel) || is_supported_rar(&rel) {
            let out = extract_dir.join(entry_name(&rel));
            if let Ok(next) = h.extract_to(&out) {
                nested_archives.push(out.to_string_lossy().to_string());
                archive = next;
            } else { break; }
        } else {
            let Ok(next) = h.skip() else { break; };
            archive = next;
        }
        current += 1;
        if last_sent.elapsed().as_millis() >= 100 || current == total {
            let _ = ch.send(Phase1Event::Progress { current, total });
            last_sent = std::time::Instant::now();
        }
    }
    sort_by_path(&mut images);
    let _ = ch.send(Phase1Event::Done { images, nested_archives });
}

// ─── フェーズ2：内側ストリーミング展開 ──────────────────────────────────────

fn stream_zip(bytes: Vec<u8>, extract_dir: &Path, ch: &tauri::ipc::Channel<ExtractedFile>) -> Result<(), String> {
    let mut archive = ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| format!("Invalid archive: {e}"))?;

    for i in 0..archive.len() {
        let mut f = archive.by_index(i).map_err(|e| format!("Read entry: {e}"))?;
        if f.is_dir() { continue; }
        let Some(enc) = f.enclosed_name().map(|p| p.to_owned()) else { continue; };
        if !is_supported_image(&enc) { continue; }

        let out = make_outpath(&enc, extract_dir)?;
        let mut of = fs::File::create(&out).map_err(|e| format!("Create: {e}"))?;
        std::io::copy(&mut f, &mut of).map_err(|e| format!("Copy: {e}"))?;
        send_image(&out, &enc, f.size(), ch)?;
    }
    Ok(())
}

fn stream_rar(path: &Path, extract_dir: &Path, ch: &tauri::ipc::Channel<ExtractedFile>) -> Result<(), String> {
    let mut archive = Archive::new(path)
        .open_for_processing()
        .map_err(|e| format!("Open rar: {e}"))?;

    loop {
        let Some(h) = archive.read_header().map_err(|e| format!("Read header: {e}"))? else { break; };
        if !h.entry().is_file() {
            archive = h.skip().map_err(|e| format!("Skip: {e}"))?;
            continue;
        }
        let fname = h.entry().filename.clone();
        let size = h.entry().unpacked_size;
        let Some(rel) = safe_relative_path(&fname) else {
            archive = h.skip().map_err(|e| format!("Skip: {e}"))?;
            continue;
        };

        if is_supported_image(&rel) {
            let out = make_outpath(&rel, extract_dir)?;
            archive = h.extract_to(&out).map_err(|e| format!("Extract: {e}"))?;
            send_image(&out, &rel, size, ch)?;
        } else {
            archive = h.skip().map_err(|e| format!("Skip: {e}"))?;
        }
    }
    Ok(())
}

// ─── 旧来の再帰展開関数（extract_archive / extract_rar / handle_file_drop 用）─

fn extract_archive_bytes(bytes: Vec<u8>, extract_dir: &Path, recurse: bool) -> Result<Vec<ExtractedFile>, String> {
    let mut archive = ZipArchive::new(Cursor::new(bytes))
        .map_err(|err| format!("Invalid archive: {err}"))?;
    let mut extracted = Vec::new();

    for index in 0..archive.len() {
        let mut file = archive.by_index(index)
            .map_err(|err| format!("Failed to read archive entry: {err}"))?;
        if file.is_dir() { continue; }
        let Some(enclosed) = file.enclosed_name().map(|path| path.to_owned()) else { continue; };

        if is_supported_image(&enclosed) {
            let out = make_outpath(&enclosed, extract_dir)?;
            let mut of = fs::File::create(&out).map_err(|err| format!("Failed to create file: {err}"))?;
            std::io::copy(&mut file, &mut of).map_err(|err| format!("Failed to extract file: {err}"))?;
            push_image(&out, &enclosed, file.size(), &mut extracted);
        } else if recurse && (is_supported_archive(&enclosed) || is_supported_rar(&enclosed)) {
            let mut inner_bytes = Vec::new();
            Read::read_to_end(&mut file, &mut inner_bytes)
                .map_err(|err| format!("Failed to read nested archive: {err}"))?;
            let stem = enclosed.file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "archive".to_string());
            let sub_dir = extract_dir.join(&stem);
            fs::create_dir_all(&sub_dir).map_err(|err| format!("Failed to create sub dir: {err}"))?;
            if is_supported_archive(&enclosed) {
                let mut sub = extract_archive_bytes(inner_bytes, &sub_dir, false)?;
                extracted.append(&mut sub);
            } else {
                let rar_name = entry_name(&enclosed);
                let temp_rar = extract_dir.join(&rar_name);
                fs::write(&temp_rar, &inner_bytes).map_err(|err| format!("Failed to write nested rar: {err}"))?;
                let mut sub = extract_rar_file(&temp_rar, &sub_dir, false)?;
                let _ = fs::remove_file(&temp_rar);
                extracted.append(&mut sub);
            }
        }
    }
    Ok(extracted)
}

fn extract_rar_bytes(bytes: Vec<u8>, extract_dir: &Path, archive_name: &str) -> Result<Vec<ExtractedFile>, String> {
    let safe_name = sanitize_name(archive_name);
    let rar_path = extract_dir.join(format!("{safe_name}.rar"));
    fs::write(&rar_path, bytes).map_err(|err| format!("Failed to write temp rar: {err}"))?;
    extract_rar_file(&rar_path, extract_dir, true)
}

fn extract_rar_file(path: &Path, extract_dir: &Path, recurse: bool) -> Result<Vec<ExtractedFile>, String> {
    let mut archive = Archive::new(path)
        .open_for_processing()
        .map_err(|err| format!("Failed to open rar: {err}"))?;
    let mut extracted = Vec::new();

    loop {
        let Some(h) = archive.read_header().map_err(|err| format!("Failed to read rar header: {err}"))? else { break; };
        if !h.entry().is_file() {
            archive = h.skip().map_err(|err| format!("Failed to skip rar entry: {err}"))?;
            continue;
        }
        let fname = h.entry().filename.clone();
        let size = h.entry().unpacked_size;
        let Some(rel) = safe_relative_path(&fname) else {
            archive = h.skip().map_err(|err| format!("Failed to skip rar entry: {err}"))?;
            continue;
        };

        if is_supported_image(&rel) {
            let out = make_outpath(&rel, extract_dir)?;
            archive = h.extract_to(out.as_path()).map_err(|err| format!("Failed to extract rar entry: {err}"))?;
            push_image(&out, &rel, size, &mut extracted);
        } else if recurse && (is_supported_archive(&rel) || is_supported_rar(&rel)) {
            let stem = rel.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "archive".to_string());
            let sub_dir = extract_dir.join(&stem);
            fs::create_dir_all(&sub_dir).map_err(|err| format!("Failed to create sub dir: {err}"))?;
            let temp_path = extract_dir.join(rel.file_name().unwrap_or(rel.as_os_str()));
            archive = h.extract_to(&temp_path).map_err(|err| format!("Failed to extract nested archive: {err}"))?;
            let mut sub = if is_supported_archive(&rel) {
                let bytes = fs::read(&temp_path).map_err(|err| format!("Failed to read nested zip: {err}"))?;
                extract_archive_bytes(bytes, &sub_dir, false)?
            } else {
                extract_rar_file(&temp_path, &sub_dir, false)?
            };
            let _ = fs::remove_file(&temp_path);
            extracted.append(&mut sub);
        } else {
            archive = h.skip().map_err(|err| format!("Failed to skip rar entry: {err}"))?;
        }
    }
    sort_by_path(&mut extracted);
    Ok(extracted)
}

// ─── コマンド ────────────────────────────────────────────────────────────────

#[command]
fn extract_archive(
    state: State<ExtractState>,
    archive_name: String,
    bytes: Vec<u8>,
) -> Result<Vec<ExtractedFile>, String> {
    // セッション内のクリアは行わない（clear_session コマンドで一括管理）
    let extract_dir = create_extract_dir(&state, &archive_name)?;
    let mut extracted = extract_archive_bytes(bytes, &extract_dir, true)?;
    sort_by_path(&mut extracted);
    Ok(extracted)
}

#[command]
async fn extract_archive_with_nested(
    state: State<'_, ExtractState>,
    archive_name: String,
    bytes: Vec<u8>,
    channel: tauri::ipc::Channel<Phase1Event>,
) -> Result<(), String> {
    // セッション内のクリアは行わない（clear_session コマンドで一括管理）
    let extract_dir = create_extract_dir(&state, &archive_name)?;
    tauri::async_runtime::spawn_blocking(move || {
        run_archive_outer_zip(bytes, &extract_dir, &channel);
    });
    Ok(())
}

#[command]
async fn extract_rar_with_nested(
    state: State<'_, ExtractState>,
    archive_name: String,
    bytes: Vec<u8>,
    channel: tauri::ipc::Channel<Phase1Event>,
) -> Result<(), String> {
    // セッション内のクリアは行わない（clear_session コマンドで一括管理）
    let extract_dir = create_extract_dir(&state, &archive_name)?;
    let safe_name = sanitize_name(&archive_name);
    let rar_path = extract_dir.join(format!("{safe_name}.rar"));
    fs::write(&rar_path, &bytes).map_err(|err| format!("Failed to write temp rar: {err}"))?;
    tauri::async_runtime::spawn_blocking(move || {
        run_archive_outer_rar(rar_path, &extract_dir, &channel);
    });
    Ok(())
}

#[command]
async fn extract_archive_from_path(
    state: State<'_, ExtractState>,
    path: String,
    channel: tauri::ipc::Channel<Phase1Event>,
) -> Result<(), String> {
    let archive_path = PathBuf::from(&path);
    let name = archive_path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "archive".to_string());
    let stem = archive_path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "archive".to_string());
    // セッション内のクリアは行わない（clear_session コマンドで一括管理）
    let extract_dir = create_extract_dir(&state, &name)?;
    let extract_subdir = extract_dir.join(&stem);
    fs::create_dir_all(&extract_subdir).map_err(|e| format!("Create subdir: {e}"))?;
    tauri::async_runtime::spawn_blocking(move || {
        match fs::read(&archive_path) {
            Ok(bytes) => run_archive_outer_zip(bytes, &extract_subdir, &channel),
            Err(e) => { let _ = channel.send(Phase1Event::Error { message: format!("Read: {e}") }); }
        }
    });
    Ok(())
}

#[command]
async fn extract_rar_from_path(
    state: State<'_, ExtractState>,
    path: String,
    channel: tauri::ipc::Channel<Phase1Event>,
) -> Result<(), String> {
    let archive_path = PathBuf::from(&path);
    let name = archive_path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "archive".to_string());
    let stem = archive_path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "archive".to_string());
    // セッション内のクリアは行わない（clear_session コマンドで一括管理）
    let extract_dir = create_extract_dir(&state, &name)?;
    let extract_subdir = extract_dir.join(&stem);
    fs::create_dir_all(&extract_subdir).map_err(|e| format!("Create subdir: {e}"))?;
    tauri::async_runtime::spawn_blocking(move || {
        run_archive_outer_rar(archive_path, &extract_subdir, &channel);
    });
    Ok(())
}

#[command]
fn extract_nested_streaming(
    archive_path: String,
    channel: tauri::ipc::Channel<ExtractedFile>,
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
fn extract_rar(
    state: State<ExtractState>,
    archive_name: String,
    bytes: Vec<u8>,
) -> Result<Vec<ExtractedFile>, String> {
    // セッション内のクリアは行わない（clear_session コマンドで一括管理）
    let extract_dir = create_extract_dir(&state, &archive_name)?;
    let extracted = extract_rar_bytes(bytes, &extract_dir, &archive_name)?;
    Ok(extracted)
}

#[command]
fn handle_file_drop(
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

fn settings_path(app: &AppHandle) -> Result<PathBuf, String> {
    let exe_path = std::env::current_exe().map_err(|err| format!("Failed to resolve current exe: {err}"))?;
    if let Some(dir) = exe_path.parent() { return Ok(dir.join("settings.ini")); }
    let dir = app.path().executable_dir().map_err(|err| format!("Failed to resolve executable directory: {err}"))?;
    Ok(dir.join("settings.ini"))
}

#[command]
fn load_settings(app: AppHandle) -> Result<Option<String>, String> {
    let path = settings_path(&app)?;
    debug_log!("load_settings: path={}", path.display());
    if !path.exists() { return Ok(None); }
    let contents = fs::read_to_string(&path).map_err(|err| format!("Failed to read settings: {err}"))?;
    debug_log!("load_settings: loaded {} bytes", contents.len());
    Ok(Some(contents))
}

#[command]
fn save_settings(app: AppHandle, contents: String) -> Result<(), String> {
    let path = settings_path(&app)?;
    debug_log!("save_settings: path={} bytes={}", path.display(), contents.len());
    fs::write(path, contents).map_err(|err| format!("Failed to save settings: {err}"))?;
    debug_log!("save_settings: done");
    Ok(())
}

// フォルダをスキャンして画像・アーカイブのソート済みリストを返す
#[command]
fn scan_directory(path: String) -> Result<Vec<FolderEntry>, String> {
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

// ZIP の中身をトップレベルにフラット展開（ディレクトリ構造は無視、内側ZIPは開かない）
fn extract_zip_to_folder(bytes: Vec<u8>, extract_dir: &Path) -> Result<(), String> {
    let mut archive = ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| format!("Invalid zip: {e}"))?;
    for i in 0..archive.len() {
        let mut f = archive.by_index(i).map_err(|e| format!("Read entry: {e}"))?;
        if f.is_dir() { continue; }
        let Some(enc) = f.enclosed_name().map(|p| p.to_owned()) else { continue; };
        let ok = is_supported_image(&enc)
            || is_supported_archive(&enc)
            || is_supported_rar(&enc)
            || is_supported_pdf(&enc);
        if !ok { continue; }
        // ファイル名のみ使用（ディレクトリ構造を無視）
        let file_name = entry_name(&enc);
        let out = extract_dir.join(&file_name);
        let mut of = fs::File::create(&out).map_err(|e| format!("Create: {e}"))?;
        std::io::copy(&mut f, &mut of).map_err(|e| format!("Copy: {e}"))?;
    }
    Ok(())
}

// RAR の中身をトップレベルにフラット展開（ディレクトリ構造は無視、内側ZIPは開かない）
fn extract_rar_to_folder(path: &Path, extract_dir: &Path) -> Result<(), String> {
    let mut archive = Archive::new(path).open_for_processing()
        .map_err(|e| format!("Open rar: {e}"))?;
    loop {
        let Some(h) = archive.read_header().map_err(|e| format!("Read header: {e}"))? else { break; };
        if !h.entry().is_file() {
            archive = h.skip().map_err(|e| format!("Skip: {e}"))?;
            continue;
        }
        let fname = h.entry().filename.clone();
        let Some(rel) = safe_relative_path(&fname) else {
            archive = h.skip().map_err(|e| format!("Skip: {e}"))?;
            continue;
        };
        let ok = is_supported_image(&rel)
            || is_supported_archive(&rel)
            || is_supported_rar(&rel)
            || is_supported_pdf(&rel);
        if ok {
            // ファイル名のみ使用（ディレクトリ構造を無視）
            let file_name = entry_name(&rel);
            let out = extract_dir.join(&file_name);
            if let Some(parent) = out.parent() {
                fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
            }
            archive = h.extract_to(&out).map_err(|e| format!("Extract: {e}"))?;
        } else {
            archive = h.skip().map_err(|e| format!("Skip: {e}"))?;
        }
    }
    Ok(())
}

// アーカイブを temp に展開してフォルダパスを返す（ZIP = フォルダと同義）
// 呼び出し側は scan_directory でフォルダとして扱う
#[command]
async fn extract_to_temp(
    state: State<'_, ExtractState>,
    archive_path: String,
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
            extract_rar_to_folder(&archive, &extract_dir)
        } else {
            fs::read(&archive)
                .map_err(|e| format!("Read: {e}"))
                .and_then(|bytes| extract_zip_to_folder(bytes, &extract_dir))
        }
    }).await.map_err(|e| format!("Task: {e}"))??;

    Ok(folder_path)
}

#[command]
fn clear_session(state: State<ExtractState>) -> Result<(), String> {
    clear_session_dirs(&state)
}

// セッション切り替え時に特定の一時フォルダを削除
#[command]
fn delete_single_temp_dir(state: State<ExtractState>, path: String) -> Result<(), String> {
    let dir = PathBuf::from(&path);
    if let Ok(mut guard) = state.session_dirs.lock() {
        guard.retain(|d| d != &dir);
    }
    cleanup_temp_dir(&dir);
    Ok(())
}

#[command]
fn list_directory(path: String) -> Result<Vec<String>, String> {
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

#[command]
fn toggle_fullscreen(window: Window) -> Result<bool, String> {
    let is_fullscreen = window.is_fullscreen().map_err(|err| format!("Failed to read fullscreen state: {err}"))?;
    let next = !is_fullscreen;
    window.set_fullscreen(next).map_err(|err| format!("Failed to set fullscreen: {err}"))?;
    Ok(next)
}

// ─── アップスケール ─────────────────────────────────────────────────────────

const NCNN_DOWNLOAD_URL: &str = "https://github.com/nihui/realcugan-ncnn-vulkan/releases/download/20220728/realcugan-ncnn-vulkan-20220728-windows.zip";
const NCNN_ZIP_PREFIX: &str = "realcugan-ncnn-vulkan-20220728-windows";

fn find_ncnn_dir(app: &AppHandle) -> Option<PathBuf> {
    if let Ok(dir) = app.path().app_data_dir() {
        let d = dir.join("realcugan-ncnn-vulkan");
        if d.join("realcugan-ncnn-vulkan.exe").exists() { return Some(d); }
    }
    if let Ok(dir) = app.path().resource_dir() {
        let d = dir.join("realcugan-ncnn-vulkan");
        if d.join("realcugan-ncnn-vulkan.exe").exists() { return Some(d); }
    }
    None
}

#[command]
fn download_ncnn_vulkan(app: AppHandle) -> Result<(), String> {
    let data_dir = app.path().app_data_dir().map_err(|e| format!("app_data_dir: {e}"))?;
    let ncnn_dir = data_dir.join("realcugan-ncnn-vulkan");
    let models_dir = ncnn_dir.join("models-se");
    fs::create_dir_all(&models_dir).map_err(|e| format!("mkdir: {e}"))?;

    let response = ureq::get(NCNN_DOWNLOAD_URL).call().map_err(|e| format!("Download failed: {e}"))?;
    let mut zip_data = Vec::new();
    response.into_reader().read_to_end(&mut zip_data).map_err(|e| format!("Read failed: {e}"))?;

    let cursor = std::io::Cursor::new(zip_data);
    let mut archive = ZipArchive::new(cursor).map_err(|e| format!("ZIP: {e}"))?;
    let targets = [
        "realcugan-ncnn-vulkan.exe", "vcomp140.dll",
        "models-se/up2x-conservative.bin", "models-se/up2x-conservative.param",
        "models-se/up4x-conservative.bin", "models-se/up4x-conservative.param",
        "models-se/up2x-no-denoise.bin",   "models-se/up2x-no-denoise.param",
        "models-se/up4x-no-denoise.bin",   "models-se/up4x-no-denoise.param",
    ];
    for rel in &targets {
        let zip_path = format!("{NCNN_ZIP_PREFIX}/{rel}");
        let mut entry = archive.by_name(&zip_path).map_err(|e| format!("Entry {rel}: {e}"))?;
        let dest = ncnn_dir.join(rel);
        if let Some(parent) = dest.parent() { fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?; }
        let mut outfile = fs::File::create(&dest).map_err(|e| format!("Create {rel}: {e}"))?;
        std::io::copy(&mut entry, &mut outfile).map_err(|e| format!("Write {rel}: {e}"))?;
    }
    Ok(())
}

fn upscale_via_ncnn(app: &AppHandle, input_path: &str, scale: u32) -> Result<UpscaleResult, String> {
    let ncnn_dir = find_ncnn_dir(app).ok_or("NCNN_NOT_INSTALLED")?;
    let exe = ncnn_dir.join("realcugan-ncnn-vulkan.exe");
    let temp_dir = std::env::temp_dir().join("viewer-on-tauri").join("upscaled");
    fs::create_dir_all(&temp_dir).map_err(|e| format!("Create temp dir: {e}"))?;
    let stem = Path::new(input_path).file_stem().and_then(|s| s.to_str()).unwrap_or("image");
    let ts = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|e| format!("Time: {e}"))?.as_millis();
    let out_path = temp_dir.join(format!("{ts}_{stem}_{scale}x.png"));
    let status = std::process::Command::new(&exe)
        .arg("-i").arg(input_path)
        .arg("-o").arg(&out_path)
        .arg("-s").arg(scale.to_string())
        .arg("-n").arg("0")
        .arg("-m").arg("models-se")
        .current_dir(&ncnn_dir)
        .status()
        .map_err(|e| format!("Failed to run realcugan-ncnn-vulkan: {e}"))?;
    if !status.success() { return Err("realcugan-ncnn-vulkan failed".to_string()); }
    let size = fs::metadata(&out_path).map(|m| m.len()).unwrap_or(0);
    Ok(UpscaleResult { path: out_path.to_string_lossy().to_string(), size })
}

#[derive(Serialize)]
struct UpscaleResult {
    path: String,
    size: u64,
}

#[command]
fn upscale_image(
    app: AppHandle,
    cache: State<UpscaleSessionCache>,
    input_path: String,
    scale: u32,
    provider: String,
) -> Result<UpscaleResult, String> {
    if provider == "vulkan" {
        return upscale_via_ncnn(&app, &input_path, scale);
    }

    let cache_key = (scale, provider.clone());
    let mut sessions = cache.sessions.lock().map_err(|_| "Failed to lock session cache".to_string())?;

    if !sessions.contains_key(&cache_key) {
        let model_file = match scale { 4 => "realcugan_4x_conservative.onnx", _ => "realcugan_2x_conservative.onnx" };
        let model_path = app.path().resource_dir().map_err(|e| format!("resource_dir: {e}"))?.join(model_file);
        if !model_path.exists() { return Err(format!("Model not found: {}", model_path.display())); }

        let num_threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
        let mut builder = Session::builder().map_err(|e| format!("ORT builder: {e}"))?
            .with_intra_threads(num_threads).map_err(|e| format!("Set threads: {e}"))?;

        if provider == "cuda" {
            builder = builder
                .with_execution_providers([ort::ep::CUDA::default().build()])
                .map_err(|e| format!("CUDA EP: {e}"))?;
        }

        let session = builder.commit_from_file(&model_path).map_err(|e| format!("Load model: {e}"))?;
        sessions.insert(cache_key.clone(), session);
    }

    let session = sessions.get_mut(&cache_key).ok_or("Session not found in cache")?;

    let img = image::open(&input_path).map_err(|e| format!("Cannot open image: {e}"))?.into_rgb8();
    let (orig_w, orig_h) = img.dimensions();
    let pad_h = ((orig_h + 1) & !1) as usize;
    let pad_w = ((orig_w + 1) & !1) as usize;

    let mut data = vec![0f32; 3 * pad_h * pad_w];
    for y in 0..orig_h as usize {
        for x in 0..orig_w as usize {
            let p = img.get_pixel(x as u32, y as u32);
            data[0 * pad_h * pad_w + y * pad_w + x] = p[0] as f32 / 255.0;
            data[1 * pad_h * pad_w + y * pad_w + x] = p[1] as f32 / 255.0;
            data[2 * pad_h * pad_w + y * pad_w + x] = p[2] as f32 / 255.0;
        }
    }
    if (orig_w as usize) < pad_w {
        for y in 0..orig_h as usize {
            for c in 0..3usize {
                data[c * pad_h * pad_w + y * pad_w + orig_w as usize] =
                    data[c * pad_h * pad_w + y * pad_w + orig_w as usize - 1];
            }
        }
    }
    if (orig_h as usize) < pad_h {
        for x in 0..pad_w {
            for c in 0..3usize {
                data[c * pad_h * pad_w + orig_h as usize * pad_w + x] =
                    data[c * pad_h * pad_w + (orig_h as usize - 1) * pad_w + x];
            }
        }
    }

    let tensor = OrtTensor::<f32>::from_array(([1usize, 3, pad_h, pad_w], data))
        .map_err(|e| format!("Create tensor: {e}"))?;

    let outputs = session.run(ort::inputs!["input" => tensor]).map_err(|e| format!("Inference: {e}"))?;

    let (out_shape, out_data) = outputs["output"].try_extract_tensor::<f32>()
        .map_err(|e| format!("Extract output: {e}"))?;

    let out_h_total = out_shape[2] as usize;
    let out_w_total = out_shape[3] as usize;
    let out_h = (orig_h * scale) as usize;
    let out_w = (orig_w * scale) as usize;
    let mut out_img = image::RgbImage::new(out_w as u32, out_h as u32);
    for y in 0..out_h {
        for x in 0..out_w {
            let r = (out_data[y * out_w_total + x] * 255.0).round().clamp(0.0, 255.0) as u8;
            let g = (out_data[out_h_total * out_w_total + y * out_w_total + x] * 255.0).round().clamp(0.0, 255.0) as u8;
            let b = (out_data[2 * out_h_total * out_w_total + y * out_w_total + x] * 255.0).round().clamp(0.0, 255.0) as u8;
            out_img.put_pixel(x as u32, y as u32, image::Rgb([r, g, b]));
        }
    }

    let temp_dir = std::env::temp_dir().join("viewer-on-tauri").join("upscaled");
    fs::create_dir_all(&temp_dir).map_err(|e| format!("Create temp dir: {e}"))?;
    let stem = Path::new(&input_path).file_stem().and_then(|s| s.to_str()).unwrap_or("image");
    let ts = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|e| format!("Time: {e}"))?.as_millis();
    let out_path = temp_dir.join(format!("{ts}_{stem}_{scale}x.png"));
    out_img.save(&out_path).map_err(|e| format!("Save: {e}"))?;
    let size = fs::metadata(&out_path).map(|m| m.len()).unwrap_or(0);
    Ok(UpscaleResult { path: out_path.to_string_lossy().to_string(), size })
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
            download_ncnn_vulkan
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

