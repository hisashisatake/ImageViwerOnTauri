/// 超解像モデル系統ごとの差異(モデルファイル名、テンソル入出力名、正規化方法、
/// パディング要件、出力チャンネル順序など)を吸収するためのプラグインインターフェース。
/// タイル分割やセッション管理などの共通処理は呼び出し側(`upscale::upscale_image`)が担い、
/// モデル固有の前処理・後処理だけをこのトレイトの実装に委ねる。
pub trait UpscaleModel: Send + Sync {
    /// 倍率に対応するモデルファイル名を返す。
    fn model_file(&self, scale: u32) -> &'static str;
    /// 推論セッションへの入力テンソル名。
    fn input_name(&self) -> &'static str;
    /// 推論セッションからの出力テンソル名。
    fn output_name(&self) -> &'static str;

    /// 画像中の矩形領域(タイル)を、モデル入力用のCHW・f32テンソルデータへ変換する。
    /// モデルが要求する次元制約(偶数幅・高さなど)を満たすようパディングを行い、
    /// 戻り値としてテンソルデータと、パディング後の幅・高さを返す。
    fn encode_region(
        &self,
        img: &image::RgbImage,
        x0: usize,
        y0: usize,
        w: usize,
        h: usize,
    ) -> (Vec<f32>, usize, usize);

    /// 推論結果テンソル(`out_data`、形状は `out_h_total` x `out_w_total` のCHW)から、
    /// 出力テンソル内座標 `(out_x, out_y)` の画素を取り出す。
    fn decode_pixel(
        &self,
        out_data: &[f32],
        out_h_total: usize,
        out_w_total: usize,
        out_x: usize,
        out_y: usize,
    ) -> image::Rgb<u8>;
}
