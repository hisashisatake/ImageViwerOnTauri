/// 超解像モデル系統ごとの差異(モデルファイル名、テンソル入出力名、正規化方法、
/// パディング要件、出力チャンネル順序など)を吸収するためのプラグインインターフェース。
/// タイル分割やセッション管理などの共通処理は呼び出し側(`upscale::upscale_image`)が担い、
/// モデル固有の前処理・後処理だけをこのトレイトの実装に委ねる。
pub trait UpscaleModel: Send + Sync {
    /// 倍率に対応するモデルファイル名を返す。
    fn model_file(&self, scale: u32) -> &'static str;
    /// このモデルが実際に出力する倍率。
    /// RealCUGANのように `scale` 引数でモデルを切り替えられるものはそれをそのまま返すが、
    /// descreentonのように倍率が固定のモデルでは要求された `scale` に関わらず固定値を返す。
    /// タイル分割側のクロップ位置・出力画像サイズの計算は、要求倍率ではなく必ずこちらを
    /// 基準にしないと、出力テンソルとの対応がずれて画像が破綻する。
    fn output_scale(&self, scale: u32) -> u32;
    /// 推論セッションへの入力テンソル名。
    fn input_name(&self) -> &'static str;
    /// 推論セッションからの出力テンソル名。
    fn output_name(&self) -> &'static str;

    /// タイル分割の基準サイズ(コア領域の一辺)。モデルが要求する入力形状の制約
    /// (固定形状エクスポートなど)によって、扱える最大タイルサイズが変わるため
    /// モデルごとに定義する。
    fn tile_size(&self) -> usize;
    /// タイル境界の継ぎ目を吸収するためのオーバーラップ幅。
    /// `tile_size() + 2 * tile_overlap()` が、`encode_region` に渡されうる
    /// 拡張領域の最大サイズになる。
    fn tile_overlap(&self) -> usize;

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
