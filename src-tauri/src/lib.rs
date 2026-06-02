use ort::{session::Session, value::Tensor as OrtTensor};
use serde::Serialize;
use std::{
    collections::HashMap,
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::{command, AppHandle, Manager, State, Window};
use unrar::Archive;
use zip::ZipArchive;

#[derive(Default)]
struct ExtractState {
    last_temp_dir: Mutex<Option<PathBuf>>,
}

#[derive(Default)]
struct UpscaleSessionCache {
    sessions: Mutex<HashMap<(u32, String), Session>>,
}

#[derive(Serialize)]
struct ExtractedFile {
    path: String,
    name: String,
    size: u64,
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

fn clear_last_temp_dir(state: &State<ExtractState>) -> Result<(), String> {
    let mut last_dir_guard = state
        .last_temp_dir
        .lock()
        .map_err(|_| "Failed to lock state".to_string())?;
    if let Some(prev_dir) = last_dir_guard.take() {
        let _ = fs::remove_dir_all(prev_dir);
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
        .last_temp_dir
        .lock()
        .map_err(|_| "Failed to lock state".to_string())?;
    *last_dir_guard = Some(extract_dir.clone());
    Ok(extract_dir)
}

fn extract_archive_bytes(bytes: Vec<u8>, extract_dir: &Path) -> Result<Vec<ExtractedFile>, String> {
    let reader = Cursor::new(bytes);
    let mut archive = ZipArchive::new(reader)
        .map_err(|err| format!("Invalid archive: {err}"))?;
    let mut extracted = Vec::new();

    for index in 0..archive.len() {
        let mut file = archive
            .by_index(index)
            .map_err(|err| format!("Failed to read archive entry: {err}"))?;

        if file.is_dir() {
            continue;
        }

        let Some(enclosed) = file.enclosed_name().map(|path| path.to_owned()) else {
            continue;
        };
        if !is_supported_image(&enclosed) {
            continue;
        }

        let out_path = extract_dir.join(&enclosed);
        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|err| format!("Failed to create output dir: {err}"))?;
        }

        let mut outfile = fs::File::create(&out_path)
            .map_err(|err| format!("Failed to create file: {err}"))?;
        std::io::copy(&mut file, &mut outfile)
            .map_err(|err| format!("Failed to extract file: {err}"))?;

        extracted.push(ExtractedFile {
            path: out_path.to_string_lossy().to_string(),
            name: enclosed
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| "image".to_string()),
            size: file.size(),
        });
    }

    Ok(extracted)
}

fn extract_rar_bytes(
    bytes: Vec<u8>,
    extract_dir: &Path,
    archive_name: &str,
) -> Result<Vec<ExtractedFile>, String> {
    let safe_name = sanitize_name(archive_name);
    let rar_path = extract_dir.join(format!("{safe_name}.rar"));
    fs::write(&rar_path, bytes).map_err(|err| format!("Failed to write temp rar: {err}"))?;
    extract_rar_file(&rar_path, extract_dir)
}

fn extract_rar_file(path: &Path, extract_dir: &Path) -> Result<Vec<ExtractedFile>, String> {
    let mut archive = Archive::new(path)
        .open_for_processing()
        .map_err(|err| format!("Failed to open rar: {err}"))?;
    let mut extracted = Vec::new();

    loop {
        let header = archive
            .read_header()
            .map_err(|err| format!("Failed to read rar header: {err}"))?;
        let Some(header) = header else {
            break;
        };
        let entry_is_file = header.entry().is_file();
        let entry_filename = header.entry().filename.clone();
        let entry_size = header.entry().unpacked_size;

        if !entry_is_file {
            archive = header
                .skip()
                .map_err(|err| format!("Failed to skip rar entry: {err}"))?;
            continue;
        }

        let Some(rel_path) = safe_relative_path(&entry_filename) else {
            archive = header
                .skip()
                .map_err(|err| format!("Failed to skip rar entry: {err}"))?;
            continue;
        };

        if !is_supported_image(&rel_path) {
            archive = header
                .skip()
                .map_err(|err| format!("Failed to skip rar entry: {err}"))?;
            continue;
        }

        let out_path = extract_dir.join(&rel_path);
        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|err| format!("Failed to create output dir: {err}"))?;
        }

        archive = header
            .extract_to(out_path.as_path())
            .map_err(|err| format!("Failed to extract rar entry: {err}"))?;

        extracted.push(ExtractedFile {
            path: out_path.to_string_lossy().to_string(),
            name: rel_path
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| "image".to_string()),
            size: entry_size,
        });
    }

    extracted.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(extracted)
}

#[command]
fn extract_archive(
    state: State<ExtractState>,
    archive_name: String,
    bytes: Vec<u8>,
) -> Result<Vec<ExtractedFile>, String> {
    clear_last_temp_dir(&state)?;
    let extract_dir = create_extract_dir(&state, &archive_name)?;
    let mut extracted = extract_archive_bytes(bytes, &extract_dir)?;
    extracted.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(extracted)
}

#[command]
fn extract_rar(
    state: State<ExtractState>,
    archive_name: String,
    bytes: Vec<u8>,
) -> Result<Vec<ExtractedFile>, String> {
    clear_last_temp_dir(&state)?;
    let extract_dir = create_extract_dir(&state, &archive_name)?;
    let extracted = extract_rar_bytes(bytes, &extract_dir, &archive_name)?;
    Ok(extracted)
}

#[command]
fn handle_file_drop(
    state: State<ExtractState>,
    paths: Vec<String>,
) -> Result<Vec<ExtractedFile>, String> {
    clear_last_temp_dir(&state)?;
    let mut extracted = Vec::new();
    let mut archive_dir: Option<PathBuf> = None;

    for path_str in paths {
        let path = PathBuf::from(&path_str);
        if !path.is_file() {
            continue;
        }

        if is_supported_image(&path) {
            let metadata = fs::metadata(&path)
                .map_err(|err| format!("Failed to read metadata: {err}"))?;
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| "image".to_string());
            extracted.push(ExtractedFile {
                path: path.to_string_lossy().to_string(),
                name,
                size: metadata.len(),
            });
            continue;
        }

        if is_supported_archive(&path) {
            let bytes = fs::read(&path)
                .map_err(|err| format!("Failed to read archive: {err}"))?;
            let dir = if let Some(existing) = &archive_dir {
                existing.clone()
            } else {
                let name = path
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_else(|| "archive".to_string());
                let new_dir = create_extract_dir(&state, &name)?;
                archive_dir = Some(new_dir.clone());
                new_dir
            };
            let mut archive_items = extract_archive_bytes(bytes, &dir)?;
            extracted.append(&mut archive_items);
        }

        if is_supported_rar(&path) {
            let dir = if let Some(existing) = &archive_dir {
                existing.clone()
            } else {
                let name = path
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_else(|| "archive".to_string());
                let new_dir = create_extract_dir(&state, &name)?;
                archive_dir = Some(new_dir.clone());
                new_dir
            };
            let mut archive_items = extract_rar_file(&path, &dir)?;
            extracted.append(&mut archive_items);
        }

        if is_supported_pdf(&path) {
            let metadata = fs::metadata(&path)
                .map_err(|err| format!("Failed to read metadata: {err}"))?;
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| "document".to_string());
            extracted.push(ExtractedFile {
                path: path.to_string_lossy().to_string(),
                name,
                size: metadata.len(),
            });
        }
    }

    extracted.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(extracted)
}

fn settings_path(app: &AppHandle) -> Result<PathBuf, String> {
    let exe_path = std::env::current_exe()
        .map_err(|err| format!("Failed to resolve current exe: {err}"))?;
    if let Some(dir) = exe_path.parent() {
        return Ok(dir.join("settings.ini"));
    }
    let dir = app
        .path()
        .executable_dir()
        .map_err(|err| format!("Failed to resolve executable directory: {err}"))?;
    Ok(dir.join("settings.ini"))
}

#[command]
fn load_settings(app: AppHandle) -> Result<Option<String>, String> {
    let path = settings_path(&app)?;
    println!("load_settings: path={}", path.display());
    if !path.exists() {
        return Ok(None);
    }
    let contents = fs::read_to_string(&path)
        .map_err(|err| format!("Failed to read settings: {err}"))?;
    println!("load_settings: loaded {} bytes", contents.len());
    Ok(Some(contents))
}

#[command]
fn save_settings(app: AppHandle, contents: String) -> Result<(), String> {
    let path = settings_path(&app)?;
    println!("save_settings: path={} bytes={}", path.display(), contents.len());
    fs::write(path, contents).map_err(|err| format!("Failed to save settings: {err}"))?;
    println!("save_settings: done");
    Ok(())
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
    let cache_key = (scale, provider.clone());
    let mut sessions = cache
        .sessions
        .lock()
        .map_err(|_| "Failed to lock session cache".to_string())?;

    if !sessions.contains_key(&cache_key) {
        let model_file = match scale {
            4 => "realcugan_4x_conservative.onnx",
            _ => "realcugan_2x_conservative.onnx",
        };
        let model_path = app
            .path()
            .resource_dir()
            .map_err(|e| format!("resource_dir: {e}"))?
            .join(model_file);

        if !model_path.exists() {
            return Err(format!("Model not found: {}", model_path.display()));
        }

        let num_threads = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);

        let mut builder = Session::builder()
            .map_err(|e| format!("ORT builder: {e}"))?
            .with_intra_threads(num_threads)
            .map_err(|e| format!("Set threads: {e}"))?;

        if provider == "cuda" {
            builder = builder
                .with_execution_providers([ort::ep::CUDA::default().build()])
                .map_err(|e| format!("CUDA EP: {e}"))?;
        }

        let session = builder
            .commit_from_file(&model_path)
            .map_err(|e| format!("Load model: {e}"))?;

        sessions.insert(cache_key.clone(), session);
    }

    let session = sessions
        .get_mut(&cache_key)
        .ok_or("Session not found in cache")?;

    let img = image::open(&input_path)
        .map_err(|e| format!("Cannot open image: {e}"))?
        .into_rgb8();
    let (orig_w, orig_h) = img.dimensions();

    // モデルは偶数サイズのみ受け付けるため、奇数の場合は端を複製してパディング
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

    let outputs = session
        .run(ort::inputs!["input" => tensor])
        .map_err(|e| format!("Inference: {e}"))?;

    let (out_shape, out_data) = outputs["output"]
        .try_extract_tensor::<f32>()
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

    let stem = Path::new(&input_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("image");
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| format!("Time: {e}"))?
        .as_millis();
    let out_path = temp_dir.join(format!("{ts}_{stem}_{scale}x.png"));

    out_img.save(&out_path).map_err(|e| format!("Save: {e}"))?;
    let size = fs::metadata(&out_path).map(|m| m.len()).unwrap_or(0);
    Ok(UpscaleResult {
        path: out_path.to_string_lossy().to_string(),
        size,
    })
}

#[command]
fn toggle_fullscreen(window: Window) -> Result<bool, String> {
    let is_fullscreen = window
        .is_fullscreen()
        .map_err(|err| format!("Failed to read fullscreen state: {err}"))?;
    let next = !is_fullscreen;
    window
        .set_fullscreen(next)
        .map_err(|err| format!("Failed to set fullscreen: {err}"))?;
    Ok(next)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run(context: tauri::Context<tauri::Wry>) {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(ExtractState::default())
        .manage(UpscaleSessionCache::default())
        .invoke_handler(tauri::generate_handler![
            extract_archive,
            extract_rar,
            handle_file_drop,
            toggle_fullscreen,
            load_settings,
            save_settings,
            upscale_image
        ])
        .run(context)
        .expect("error while running tauri application");
}
