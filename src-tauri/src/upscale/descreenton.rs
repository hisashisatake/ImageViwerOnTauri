use super::model::UpscaleModel;

/// OmniSR系 "descreenton" モデル(JPEG圧縮ノイズ除去寄りの軽量4倍超解像)の実装。
///
/// `tools/omnisr-convert` でのONNX変換はトレースベースのエクスポートに依存しており、
/// モデル内部のwindow分割処理がダミー入力サイズに応じた固定形状としてグラフへ
/// 焼き込まれてしまう(=他サイズの入力に汎化しない)。そのため変換時に入力形状を
/// `FIXED_SIZE` x `FIXED_SIZE` に固定しており、推論側も常にこのサイズのテンソルを
/// 渡す必要がある。`tile_size() + 2 * tile_overlap()` を `FIXED_SIZE` に一致させる
/// ことで、タイル分割が生成する拡張領域を過不足なくこのサイズへ収める。
const FIXED_SIZE: usize = 128;

/// VL4は強い圧縮劣化、VH4は軽微な劣化のJPEG画像向けに学習された重み。
pub enum DescreentonVariant {
    Vl4,
    Vh4,
}

pub struct Descreenton(pub DescreentonVariant);

impl UpscaleModel for Descreenton {
    fn model_file(&self, _scale: u32) -> &'static str {
        match self.0 {
            DescreentonVariant::Vl4 => "descreenton_vl4.onnx",
            DescreentonVariant::Vh4 => "descreenton_vh4.onnx",
        }
    }

    // VL4/VH4ともに4倍固定のモデルであり、要求された scale には依存しない。
    fn output_scale(&self, _scale: u32) -> u32 { 4 }

    fn input_name(&self) -> &'static str { "input" }
    fn output_name(&self) -> &'static str { "output" }

    fn tile_size(&self) -> usize { 96 }
    fn tile_overlap(&self) -> usize { 16 }

    fn encode_region(
        &self,
        img: &image::RgbImage,
        x0: usize,
        y0: usize,
        w: usize,
        h: usize,
    ) -> (Vec<f32>, usize, usize) {
        let pad_w = FIXED_SIZE;
        let pad_h = FIXED_SIZE;

        let mut data = vec![0f32; 3 * pad_h * pad_w];
        for ty in 0..h {
            for tx in 0..w {
                let p = img.get_pixel((x0 + tx) as u32, (y0 + ty) as u32);
                data[0 * pad_h * pad_w + ty * pad_w + tx] = p[0] as f32 / 255.0;
                data[1 * pad_h * pad_w + ty * pad_w + tx] = p[1] as f32 / 255.0;
                data[2 * pad_h * pad_w + ty * pad_w + tx] = p[2] as f32 / 255.0;
            }
        }
        // 固定形状の入力を満たすため、右端・下端の不足分は端の画素を複製して埋める
        if w < pad_w {
            for ty in 0..h {
                for c in 0..3usize {
                    let edge = data[c * pad_h * pad_w + ty * pad_w + w - 1];
                    for tx in w..pad_w {
                        data[c * pad_h * pad_w + ty * pad_w + tx] = edge;
                    }
                }
            }
        }
        if h < pad_h {
            for tx in 0..pad_w {
                for c in 0..3usize {
                    let edge = data[c * pad_h * pad_w + (h - 1) * pad_w + tx];
                    for ty in h..pad_h {
                        data[c * pad_h * pad_w + ty * pad_w + tx] = edge;
                    }
                }
            }
        }
        (data, pad_w, pad_h)
    }

    fn decode_pixel(
        &self,
        out_data: &[f32],
        out_h_total: usize,
        out_w_total: usize,
        out_x: usize,
        out_y: usize,
    ) -> image::Rgb<u8> {
        let plane = out_h_total * out_w_total;
        let idx = out_y * out_w_total + out_x;
        let r = (out_data[idx] * 255.0).round().clamp(0.0, 255.0) as u8;
        let g = (out_data[plane + idx] * 255.0).round().clamp(0.0, 255.0) as u8;
        let b = (out_data[2 * plane + idx] * 255.0).round().clamp(0.0, 255.0) as u8;
        image::Rgb([r, g, b])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ort::{session::Session, value::Tensor as OrtTensor};

    #[test]
    fn encode_region_pads_to_fixed_size_with_edge_replication() {
        let model = Descreenton(DescreentonVariant::Vl4);
        let mut img = image::RgbImage::new(5, 3);
        for y in 0..3 {
            for x in 0..5 {
                img.put_pixel(x, y, image::Rgb([(x * 10) as u8, (y * 20) as u8, 100]));
            }
        }
        let (data, pad_w, pad_h) = model.encode_region(&img, 0, 0, 5, 3);
        assert_eq!(pad_w, FIXED_SIZE);
        assert_eq!(pad_h, FIXED_SIZE);
        assert_eq!(data.len(), 3 * FIXED_SIZE * FIXED_SIZE);

        // 右端は最終列の複製になっているはず
        let last_col = img.get_pixel(4, 0);
        assert_eq!(data[0 * pad_h * pad_w + 0 * pad_w + 5], last_col[0] as f32 / 255.0);
        // 下端も最終行の複製になっているはず
        let last_row = img.get_pixel(0, 2);
        assert_eq!(data[0 * pad_h * pad_w + 3 * pad_w + 0], last_row[0] as f32 / 255.0);
    }

    #[test]
    fn onnx_inference_roundtrip_produces_4x_output() {
        let model_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("descreenton_vl4.onnx");
        if !model_path.exists() {
            eprintln!("skip: {} not found", model_path.display());
            return;
        }
        let model = Descreenton(DescreentonVariant::Vl4);

        let mut img = image::RgbImage::new(40, 30);
        for y in 0..30 {
            for x in 0..40 {
                img.put_pixel(x, y, image::Rgb([(x * 5) as u8, (y * 5) as u8, 128]));
            }
        }

        let (data, pad_w, pad_h) = model.encode_region(&img, 0, 0, 40, 30);
        let tensor = OrtTensor::<f32>::from_array(([1usize, 3, pad_h, pad_w], data)).unwrap();

        let mut session = Session::builder().unwrap().commit_from_file(&model_path).unwrap();
        let outputs = session.run(ort::inputs![model.input_name() => tensor]).unwrap();
        let (out_shape, out_data) = outputs[model.output_name()].try_extract_tensor::<f32>().unwrap();

        let out_h_total = out_shape[2] as usize;
        let out_w_total = out_shape[3] as usize;
        assert_eq!(out_h_total, FIXED_SIZE * 4);
        assert_eq!(out_w_total, FIXED_SIZE * 4);
        // 出力テンソルにNaN/Infが出ていないことを確認する(固定形状エクスポートが
        // 壊れていれば、ここで異常値が混入するはず)
        assert!(out_data.iter().all(|v| v.is_finite()));
        let _ = model.decode_pixel(out_data, out_h_total, out_w_total, 0, 0);
    }
}
