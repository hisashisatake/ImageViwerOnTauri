use super::zip::extract_archive_bytes;
use super::{create_extract_dir, make_outpath, push_image, send_image, sort_by_path, ExtractState, ExtractedFile, Phase1Event};
use crate::media::{entry_name, is_supported_archive, is_supported_image, is_supported_pdf, is_supported_rar, safe_relative_path, sanitize_name};
use std::{
    fs,
    path::{Path, PathBuf},
};
use tauri::{command, ipc::Channel, State};
use unrar::Archive;

pub(crate) fn count_rar_entries(path: &Path) -> usize {
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

pub(super) fn run_archive_outer_rar(path: PathBuf, extract_dir: &Path, ch: &Channel<Phase1Event>) {
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

pub(super) fn stream_rar(path: &Path, extract_dir: &Path, ch: &Channel<ExtractedFile>) -> Result<(), String> {
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

// ─── 旧来の再帰展開関数（extract_rar / handle_file_drop 用）───────────────────
// rar の中に zip が、zip の中に rar がネストされうるため zip 側と相互再帰する。

fn extract_rar_bytes(bytes: Vec<u8>, extract_dir: &Path, archive_name: &str) -> Result<Vec<ExtractedFile>, String> {
    let safe_name = sanitize_name(archive_name);
    let rar_path = extract_dir.join(format!("{safe_name}.rar"));
    fs::write(&rar_path, bytes).map_err(|err| format!("Failed to write temp rar: {err}"))?;
    extract_rar_file(&rar_path, extract_dir, true)
}

pub(crate) fn extract_rar_file(path: &Path, extract_dir: &Path, recurse: bool) -> Result<Vec<ExtractedFile>, String> {
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

// RAR の中身をトップレベルにフラット展開（ディレクトリ構造は無視、内側ZIPは開かない）
pub(super) fn extract_rar_to_folder<F: FnMut(usize, usize)>(path: &Path, extract_dir: &Path, mut on_progress: F) -> Result<(), String> {
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
