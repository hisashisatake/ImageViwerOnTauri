use super::model::UpscaleModel;

/// RealCUGAN系モデル(tile_mode=0でエクスポートしたONNX)の実装。
/// 入力は0-1に正規化したCHW・f32、出力チャンネル順序はR/G/Bプレーンが連続するCHW。
/// ネットワークが偶数次元を要求するため、奇数辺は端の画素を複製してパディングする。
pub struct RealCugan;

impl UpscaleModel for RealCugan {
    fn model_file(&self, scale: u32) -> &'static str {
        match scale {
            4 => "realcugan_4x_conservative.onnx",
            _ => "realcugan_2x_conservative.onnx",
        }
    }

    fn output_scale(&self, scale: u32) -> u32 { scale }

    fn input_name(&self) -> &'static str { "input" }
    fn output_name(&self) -> &'static str { "output" }

    // 画像全体を一括推論するとメモリ消費が解像度の2乗に比例して膨らむため、
    // タイル単位で推論する。オーバーラップはモデル内部のreflectパディング
    // (18-19px)による継ぎ目を吸収するためのマージン。
    fn tile_size(&self) -> usize { 256 }
    fn tile_overlap(&self) -> usize { 32 }

    fn encode_region(
        &self,
        img: &image::RgbImage,
        x0: usize,
        y0: usize,
        w: usize,
        h: usize,
    ) -> (Vec<f32>, usize, usize) {
        let pad_w = (w + 1) & !1;
        let pad_h = (h + 1) & !1;

        let mut data = vec![0f32; 3 * pad_h * pad_w];
        for ty in 0..h {
            for tx in 0..w {
                let p = img.get_pixel((x0 + tx) as u32, (y0 + ty) as u32);
                data[0 * pad_h * pad_w + ty * pad_w + tx] = p[0] as f32 / 255.0;
                data[1 * pad_h * pad_w + ty * pad_w + tx] = p[1] as f32 / 255.0;
                data[2 * pad_h * pad_w + ty * pad_w + tx] = p[2] as f32 / 255.0;
            }
        }
        if w < pad_w {
            for ty in 0..h {
                for c in 0..3usize {
                    data[c * pad_h * pad_w + ty * pad_w + w] =
                        data[c * pad_h * pad_w + ty * pad_w + w - 1];
                }
            }
        }
        if h < pad_h {
            for tx in 0..pad_w {
                for c in 0..3usize {
                    data[c * pad_h * pad_w + h * pad_w + tx] =
                        data[c * pad_h * pad_w + (h - 1) * pad_w + tx];
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
