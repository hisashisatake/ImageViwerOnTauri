use ort::{session::Session, value::Tensor as OrtTensor};
use serde::Serialize;
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::{command, AppHandle, Manager, State};
use zip::ZipArchive;

// ONNXセッションはモデル重み＋実行プロバイダの内部バッファを保持するため1つあたり
// 数百MB級のメモリを占有しうる。無制限にキャッシュすると倍率/プロバイダの組み合わせを
// 切り替えるたびに積み上がってしまうため、LRU方式で最大保持数を制限する。
const MAX_CACHED_SESSIONS: usize = 2;

#[derive(Default)]
pub struct UpscaleSessionCache {
    // 末尾ほど最近使われたセッション（LRU）。Vecの線形探索で十分な規模(最大数件)。
    sessions: Mutex<Vec<((u32, String), Session)>>,
}

#[derive(Serialize)]
pub struct UpscaleResult {
    path: String,
    size: u64,
}

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
pub fn download_ncnn_vulkan(app: AppHandle) -> Result<(), String> {
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

#[command]
pub fn get_active_ep() -> &'static str {
    #[cfg(feature = "cuda-ep")] { return "CUDA"; }
    #[cfg(feature = "directml-ep")] { return "DirectML"; }
    #[cfg(not(any(feature = "cuda-ep", feature = "directml-ep")))] { "CPU" }
}

#[command]
pub fn upscale_image(
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

    if let Some(pos) = sessions.iter().position(|(k, _)| *k == cache_key) {
        // 最近使われたものとして末尾に移動する(LRU更新)
        let entry = sessions.remove(pos);
        sessions.push(entry);
    } else {
        let model_file = match scale { 4 => "realcugan_4x_conservative.onnx", _ => "realcugan_2x_conservative.onnx" };
        let model_path = app.path().resource_dir().map_err(|e| format!("resource_dir: {e}"))?.join(model_file);
        if !model_path.exists() { return Err(format!("Model not found: {}", model_path.display())); }

        let num_threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
        let mut builder = Session::builder().map_err(|e| format!("ORT builder: {e}"))?
            .with_intra_threads(num_threads).map_err(|e| format!("Set threads: {e}"))?;

        #[cfg(feature = "cuda-ep")]
        if provider == "gpu" {
            builder = builder.with_execution_providers([ort::ep::CUDA::default().build(), ort::ep::CPU::default().build()]).map_err(|e| format!("CUDA EP: {e}"))?;
        }

        // DirectML EPはRealCUGANモデルの計算結果が崩れる（紫色化）ため、DirectMLビルドでも
        // Upscale推論自体はCPU EPにフォールバックする（EPを追加しなければ既定でCPU実行になる）。

        let session = builder.commit_from_file(&model_path).map_err(|e| format!("Load model: {e}"))?;
        sessions.push((cache_key.clone(), session));
        // 上限を超えた分は最も使われていないもの(先頭)から破棄する
        while sessions.len() > MAX_CACHED_SESSIONS {
            sessions.remove(0);
        }
    }

    let (_, session) = sessions.last_mut().ok_or("Session not found in cache")?;

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
