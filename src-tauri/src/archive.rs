use crate::media::{
    entry_name, is_supported_archive, is_supported_image, is_supported_pdf, is_supported_rar,
    safe_relative_path, sanitize_name,
};
use serde::Serialize;
use std::{
    fs,
    io::{Cursor, Read},
    path::{Path, PathBuf},
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::{command, ipc::Channel, State};
use unrar::Archive;
use zip::ZipArchive;

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

// ─── 共通ヘルパー ───────────────────────────────────────────────────────────

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

fn run_archive_outer_zip(bytes: Vec<u8>, extract_dir: &Path, ch: &Channel<Phase1Event>) {
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

fn run_archive_outer_rar(path: PathBuf, extract_dir: &Path, ch: &Channel<Phase1Event>) {
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

fn stream_zip(bytes: Vec<u8>, extract_dir: &Path, ch: &Channel<ExtractedFile>) -> Result<(), String> {
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

fn stream_rar(path: &Path, extract_dir: &Path, ch: &Channel<ExtractedFile>) -> Result<(), String> {
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

// ZIP の中身をトップレベルにフラット展開（ディレクトリ構造は無視、内側ZIPは開かない）
fn extract_zip_to_folder<F: FnMut(usize, usize)>(bytes: Vec<u8>, extract_dir: &Path, mut on_progress: F) -> Result<(), String> {
    let mut archive = ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| format!("Invalid zip: {e}"))?;
    let total = archive.len();
    let mut last_sent = std::time::Instant::now();
    for i in 0..total {
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
        if last_sent.elapsed().as_millis() >= 100 || i + 1 == total {
            on_progress(i + 1, total);
            last_sent = std::time::Instant::now();
        }
    }
    Ok(())
}

// RAR の中身をトップレベルにフラット展開（ディレクトリ構造は無視、内側ZIPは開かない）
fn extract_rar_to_folder<F: FnMut(usize, usize)>(path: &Path, extract_dir: &Path, mut on_progress: F) -> Result<(), String> {
    let mut archive = Archive::new(path).open_for_processing()
        .map_err(|e| format!("Open rar: {e}"))?;
    let mut current = 0usize;
    let mut last_sent = std::time::Instant::now();
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
        current += 1;
        if last_sent.elapsed().as_millis() >= 100 {
            on_progress(current, 0); // total = 0 = unknown
            last_sent = std::time::Instant::now();
        }
    }
    if current > 0 { on_progress(current, 0); }
    Ok(())
}

// ─── コマンド ────────────────────────────────────────────────────────────────

#[command]
pub fn extract_archive(
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
pub async fn extract_archive_with_nested(
    state: State<'_, ExtractState>,
    archive_name: String,
    bytes: Vec<u8>,
    channel: Channel<Phase1Event>,
) -> Result<(), String> {
    // セッション内のクリアは行わない（clear_session コマンドで一括管理）
    let extract_dir = create_extract_dir(&state, &archive_name)?;
    tauri::async_runtime::spawn_blocking(move || {
        run_archive_outer_zip(bytes, &extract_dir, &channel);
    });
    Ok(())
}

#[command]
pub async fn extract_rar_with_nested(
    state: State<'_, ExtractState>,
    archive_name: String,
    bytes: Vec<u8>,
    channel: Channel<Phase1Event>,
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
pub async fn extract_archive_from_path(
    state: State<'_, ExtractState>,
    path: String,
    channel: Channel<Phase1Event>,
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
pub async fn extract_rar_from_path(
    state: State<'_, ExtractState>,
    path: String,
    channel: Channel<Phase1Event>,
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
pub fn extract_rar(
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
