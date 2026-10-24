/// 单行 OCR 结果（带屏幕坐标），供替换翻译 Canvas 覆盖层使用。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RapidOcrPoint {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RapidOcrLine {
    pub id: String,
    pub text: String,
    pub points: Vec<RapidOcrPoint>,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}
