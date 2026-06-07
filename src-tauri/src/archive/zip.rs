use super::rar::extract_rar_file;
use super::{create_extract_dir, make_outpath, push_image, send_image, sort_by_path, ExtractState, ExtractedFile, Phase1Event};
use crate::media::{entry_name, is_supported_archive, is_supported_image, is_supported_pdf, is_supported_rar};
use std::{
    fs,
    io::{Cursor, Read},
    path::{Path, PathBuf},
};
use tauri::{command, ipc::Channel, State};
use zip::ZipArchive;

pub(super) fn run_archive_outer_zip(bytes: Vec<u8>, extract_dir: &Path, ch: &Channel<Phase1Event>) {
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

pub(super) fn stream_zip(bytes: Vec<u8>, extract_dir: &Path, ch: &Channel<ExtractedFile>) -> Result<(), String> {
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

// ─── 旧来の再帰展開関数（extract_archive / handle_file_drop 用）───────────────
// zip の中に rar が、rar の中に zip がネストされうるため rar 側と相互再帰する。

pub(crate) fn extract_archive_bytes(bytes: Vec<u8>, extract_dir: &Path, recurse: bool) -> Result<Vec<ExtractedFile>, String> {
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

// ZIP の中身をトップレベルにフラット展開（ディレクトリ構造は無視、内側ZIPは開かない）
pub(super) fn extract_zip_to_folder<F: FnMut(usize, usize)>(bytes: Vec<u8>, extract_dir: &Path, mut on_progress: F) -> Result<(), String> {
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
