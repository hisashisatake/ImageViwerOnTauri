mod model;
mod real_cugan;

use model::UpscaleModel;
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

use real_cugan::RealCugan;

// ONNXセッションはモデル重み＋実行プロバイダの内部バッファを保持するため1つあたり
// 数百MB級のメモリを占有しうる。無制限にキャッシュすると倍率/プロバイダの組み合わせを
// 切り替えるたびに積み上がってしまうため、LRU方式で最大保持数を制限する。
const MAX_CACHED_SESSIONS: usize = 2;

// 画像全体を一括推論するとメモリ消費が解像度の2乗に比例して膨らむため、タイル単位で
// 推論して継ぎ合わせる。TILE_OVERLAPはモデル内部のreflectパディング(18-19px)による
// 継ぎ目を吸収するためのマージン(これより大きくしておけば縫い目が出にくい)。
const TILE_SIZE: usize = 256;
const TILE_OVERLAP: usize = 32;

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

    let model: &dyn UpscaleModel = &RealCugan;

    let cache_key = (scale, provider.clone());
    let mut sessions = cache.sessions.lock().map_err(|_| "Failed to lock session cache".to_string())?;

    if let Some(pos) = sessions.iter().position(|(k, _)| *k == cache_key) {
        // 最近使われたものとして末尾に移動する(LRU更新)
        let entry = sessions.remove(pos);
        sessions.push(entry);
    } else {
        let model_file = model.model_file(scale);
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
    let orig_w = orig_w as usize;
    let orig_h = orig_h as usize;
    let scale_usize = scale as usize;

    let mut out_img = image::RgbImage::new((orig_w * scale_usize) as u32, (orig_h * scale_usize) as u32);

    // 画像全体を一括推論せず、オーバーラップ付きのタイル単位で推論して継ぎ合わせる。
    // オーバーラップ部分は推論後に切り捨てることで、モデル内部のreflectパディングに
    // 起因するタイル境界の継ぎ目を防ぐ。
    let mut y0 = 0usize;
    while y0 < orig_h {
        let y1 = (y0 + TILE_SIZE).min(orig_h);
        let mut x0 = 0usize;
        while x0 < orig_w {
            let x1 = (x0 + TILE_SIZE).min(orig_w);

            // オーバーラップを含む拡張領域(画像の端ではクランプ)
            let ey0 = y0.saturating_sub(TILE_OVERLAP);
            let ey1 = (y1 + TILE_OVERLAP).min(orig_h);
            let ex0 = x0.saturating_sub(TILE_OVERLAP);
            let ex1 = (x1 + TILE_OVERLAP).min(orig_w);

            let tile_h = ey1 - ey0;
            let tile_w = ex1 - ex0;

            let (data, pad_w, pad_h) = model.encode_region(&img, ex0, ey0, tile_w, tile_h);

            let tensor = OrtTensor::<f32>::from_array(([1usize, 3, pad_h, pad_w], data))
                .map_err(|e| format!("Create tensor: {e}"))?;

            let outputs = session.run(ort::inputs![model.input_name() => tensor]).map_err(|e| format!("Inference: {e}"))?;

            let (out_shape, out_data) = outputs[model.output_name()].try_extract_tensor::<f32>()
                .map_err(|e| format!("Extract output: {e}"))?;

            let out_h_total = out_shape[2] as usize;
            let out_w_total = out_shape[3] as usize;

            // 拡張領域のうち、本来のタイル(コア領域)に対応する部分だけを切り出して書き込む
            let crop_top = (y0 - ey0) * scale_usize;
            let crop_left = (x0 - ex0) * scale_usize;
            let crop_h = (y1 - y0) * scale_usize;
            let crop_w = (x1 - x0) * scale_usize;

            for cy in 0..crop_h {
                for cx in 0..crop_w {
                    let sy = crop_top + cy;
                    let sx = crop_left + cx;
                    let pixel = model.decode_pixel(out_data, out_h_total, out_w_total, sx, sy);
                    out_img.put_pixel(
                        (x0 * scale_usize + cx) as u32,
                        (y0 * scale_usize + cy) as u32,
                        pixel,
                    );
                }
            }

            x0 = x1;
        }
        y0 = y1;
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
