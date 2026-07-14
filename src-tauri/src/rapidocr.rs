//! RapidOCR 离线 OCR：跨平台 PaddleOCR ONNX pipeline。
//!
//! 单一模型：PP-OCRv6 medium（50 语言，文件放模型目录 `high/` 子目录）。
//! ONNX Runtime、校验清单和可续传下载器由 [`crate::offline_models::OfflineModelManager`]
//! 管理；本模块只负责 OCR pipeline 生命周期与结果后处理。
//!
//! ONNX Runtime dylib 通过 `ort` 的 `load-dynamic` feature 在运行时加载,
//! 安装包不带任何 ONNX Runtime 二进制。

use std::path::PathBuf;
use std::sync::Arc;

use crate::offline_models::{ensure_ort_init, OfflineModelManager};
use oar_ocr::oarocr::{OAROCRBuilder, OAROCR};
use serde::Serialize;
use tokio::sync::OnceCell;

/// 前端拉状态用:就绪情况 + 模型目录(供 UI 显示)。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RapidOcrStatus {
    pub available: bool,
    pub model_dir: Option<String>,
}

/// install() 返回值:成功/失败 + 人类可读 message。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RapidOcrInstallResult {
    pub success: bool,
    pub message: String,
}

/// AppState 持有的 RapidOCR 客户端。线程安全,Tauri command 间共享。
pub struct RapidOcrClient {
    models: Arc<OfflineModelManager>,
    /// 首次调用时初始化:ort::init_from(进程级一次) + OAROCRBuilder.build,之后复用。
    pipeline: OnceCell<Arc<OAROCR>>,
}

impl RapidOcrClient {
    pub fn new(models: Arc<OfflineModelManager>) -> Arc<Self> {
        Arc::new(Self {
            models,
            pipeline: OnceCell::new(),
        })
    }

    /// 测试用:不持有 AppHandle 的占位实例。所有 OCR 操作立即 Err。
    #[cfg(test)]
    pub fn disabled() -> Arc<Self> {
        Self::new(OfflineModelManager::headless(reqwest::Client::new()))
    }

    /// 测试用:绕开 AppHandle,直接指定模型目录(供真实推理 E2E 测试落地固定缓存目录)。
    /// 所有其他行为(install/status/ocr_image)与生产路径完全一致,只是 model_dir() 来源不同。
    #[cfg(test)]
    pub fn with_model_dir(dir: PathBuf, http: reqwest::Client) -> Arc<Self> {
        Self::new(OfflineModelManager::with_model_dir(dir, http))
    }

    /// Headless (no-AppHandle) client for the `kivio-code` CLI. OCR is never
    /// invoked from the terminal agent; carries `app: None` so any OCR call
    /// errors out, identical to the test `disabled()` placeholder.
    pub fn headless(models: Arc<OfflineModelManager>) -> Arc<Self> {
        Self::new(models)
    }

    /// 模型 + dylib 落盘位置:`{app_data_dir}/rapidocr-models/`。
    fn model_dir(&self) -> Result<PathBuf, String> {
        self.models.model_dir()
    }

    /// 就绪情况。必备文件不全 → available=false,前端渲染下载按钮。
    pub fn status(&self) -> RapidOcrStatus {
        let Ok(dir) = self.model_dir() else {
            return RapidOcrStatus {
                available: false,
                model_dir: None,
            };
        };
        RapidOcrStatus {
            available: self.models.rapidocr_ready(),
            model_dir: Some(dir.to_string_lossy().into_owned()),
        }
    }

    /// 顺序下载必备文件:GET → 写 .tmp → rename。任一失败立刻返回 fail,不留半成品。
    /// install_lock 防止双击并发。
    pub async fn install(&self) -> RapidOcrInstallResult {
        let result = self.models.install_rapidocr().await;
        RapidOcrInstallResult {
            success: result.success,
            message: result.message,
        }
    }

    /// 解析模型目录 + 校验文件齐全 + 惰性初始化并复用 OAROCR pipeline。
    /// `ocr_image` / `ocr_image_lines` 共用：文件不齐 → `rapidocr_models_missing`；
    /// 首次走 OnceCell init（ort::init_from 全进程一次 + 构建 pipeline，~1-3s），后续复用。
    async fn pipeline(&self) -> Result<Arc<OAROCR>, String> {
        let dir = self.model_dir()?;
        if !self.models.rapidocr_ready() {
            return Err("rapidocr_models_missing".into());
        }

        ensure_ort_init(&self.models).await?;

        let pipeline = self
            .pipeline
            .get_or_try_init(|| async {
                let high = dir.join("high");
                let builder = OAROCRBuilder::new(
                    high.join("det.onnx").to_string_lossy().into_owned(),
                    high.join("rec.onnx").to_string_lossy().into_owned(),
                    high.join("keys.txt").to_string_lossy().into_owned(),
                )
                // v6 medium 检测阈值与 v3~v5 不同,builder 默认值(0.3/0.6/1.5)只适配旧模型,
                // 必须显式覆盖,否则漏检严重。
                .text_detection_config(oar_ocr::domain::TextDetectionConfig {
                    score_threshold: 0.2,
                    box_threshold: 0.45,
                    unclip_ratio: 1.4,
                    ..Default::default()
                });
                let p = builder
                    .build()
                    .map_err(|e| format!("OAROCRBuilder failed: {e}"))?;
                Ok::<_, String>(Arc::new(p))
            })
            .await?;
        Ok(pipeline.clone())
    }

    /// OCR 主入口。文件不齐 → `rapidocr_models_missing` 错误码,前端渲染下载提示。
    /// 首次调用走 OnceCell init:ort::init_from(进程级一次) + 构建 pipeline(~1-3s),后续复用。
    pub async fn ocr_image(
        self: &Arc<Self>,
        image_path: &std::path::Path,
    ) -> Result<String, String> {
        let pipeline = self.pipeline().await?;

        // oar-ocr 同步 API,spawn_blocking 避免阻塞 tokio 调度器。
        let path = image_path.to_owned();
        let text = tokio::task::spawn_blocking(move || -> Result<String, String> {
            let img = oar_ocr::utils::load_image(&path).map_err(|e| format!("load_image: {e}"))?;
            let results = pipeline
                .predict(vec![img])
                .map_err(|e| format!("predict: {e}"))?;
            Ok(join_text_regions(&results))
        })
        .await
        .map_err(|e| format!("spawn_blocking: {e}"))??;

        Ok(text)
    }

    /// 带 bbox 的行级 OCR，供替换翻译在原位绘制译文。与 `ocr_image` 共用 pipeline。
    pub async fn ocr_image_lines(
        self: &Arc<Self>,
        image_path: &std::path::Path,
    ) -> Result<Vec<RapidOcrLine>, String> {
        let pipeline = self.pipeline().await?;

        let path = image_path.to_owned();
        let lines = tokio::task::spawn_blocking(move || -> Result<Vec<RapidOcrLine>, String> {
            let img = oar_ocr::utils::load_image(&path).map_err(|e| format!("load_image: {e}"))?;
            let results = pipeline
                .predict(vec![img])
                .map_err(|e| format!("predict: {e}"))?;
            Ok(lines_from_results(&results))
        })
        .await
        .map_err(|e| format!("spawn_blocking: {e}"))??;

        Ok(lines)
    }
}

/// 单行 OCR 结果（带屏幕坐标），供替换翻译 Canvas 覆盖层使用。
#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RapidOcrPoint {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, Serialize)]
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

#[derive(Debug, Clone)]
struct OcrSpan {
    text: String,
    points: Vec<RapidOcrPoint>,
    x_min: f32,
    y_min: f32,
    x_max: f32,
    y_max: f32,
}

impl OcrSpan {
    fn height(&self) -> f32 {
        (self.y_max - self.y_min).max(1.0)
    }

    fn center_x(&self) -> f32 {
        (self.x_min + self.x_max) / 2.0
    }

    fn center_y(&self) -> f32 {
        (self.y_min + self.y_max) / 2.0
    }
}

#[derive(Debug, Clone)]
struct OcrLine {
    text: String,
    x_min: f32,
    y_min: f32,
    x_max: f32,
    y_max: f32,
}

impl OcrLine {
    fn from_spans(mut spans: Vec<OcrSpan>) -> Self {
        spans.sort_by(|a, b| cmp_f32(a.center_x(), b.center_x()));
        let text = join_line_spans(&spans);
        let x_min = spans.iter().map(|s| s.x_min).fold(f32::INFINITY, f32::min);
        let y_min = spans.iter().map(|s| s.y_min).fold(f32::INFINITY, f32::min);
        let x_max = spans
            .iter()
            .map(|s| s.x_max)
            .fold(f32::NEG_INFINITY, f32::max);
        let y_max = spans
            .iter()
            .map(|s| s.y_max)
            .fold(f32::NEG_INFINITY, f32::max);
        Self {
            text,
            x_min,
            y_min,
            x_max,
            y_max,
        }
    }

    fn height(&self) -> f32 {
        (self.y_max - self.y_min).max(1.0)
    }

    fn center_y(&self) -> f32 {
        (self.y_min + self.y_max) / 2.0
    }
}

/// 从 OCR 原始结果提取带 bbox 的文本块（每个 DBNet 检测框一块）。
fn collect_ocr_spans(results: &[oar_ocr::oarocr::OAROCRResult]) -> (Vec<OcrSpan>, Vec<String>) {
    let mut spans = Vec::new();
    let mut fallback = Vec::new();

    for r in results {
        for region in &r.text_regions {
            let Some(text) = region.text.as_ref() else {
                continue;
            };
            let s = text.trim();
            if s.is_empty() {
                continue;
            }

            let bbox = &region.bounding_box;
            if bbox.points.is_empty() {
                fallback.push(s.to_string());
                continue;
            }

            let x_min = bbox.x_min();
            let y_min = bbox.y_min();
            let x_max = bbox.x_max();
            let y_max = bbox.y_max();
            if x_max <= x_min || y_max <= y_min {
                fallback.push(s.to_string());
                continue;
            }

            spans.push(OcrSpan {
                text: s.to_string(),
                points: bbox
                    .points
                    .iter()
                    .map(|point| RapidOcrPoint {
                        x: point.x,
                        y: point.y,
                    })
                    .collect(),
                x_min,
                y_min,
                x_max,
                y_max,
            });
        }
    }

    (spans, fallback)
}

/// 从 OCR 原始结果提取带 bbox 的视觉行（共享几何聚合逻辑）。
fn collect_ocr_lines(results: &[oar_ocr::oarocr::OAROCRResult]) -> (Vec<OcrLine>, Vec<String>) {
    let (mut spans, fallback) = collect_ocr_spans(results);

    if spans.is_empty() {
        return (Vec::new(), fallback);
    }

    let median_height = median(spans.iter().map(OcrSpan::height).collect()).unwrap_or(20.0);
    spans.sort_by(|a, b| cmp_f32(a.center_y(), b.center_y()).then(cmp_f32(a.x_min, b.x_min)));

    let mut grouped: Vec<Vec<OcrSpan>> = Vec::new();
    for span in spans {
        let same_line = grouped
            .last()
            .is_some_and(|line| span_belongs_to_line(line, &span, median_height));
        if same_line {
            grouped.last_mut().expect("line exists").push(span);
        } else {
            grouped.push(vec![span]);
        }
    }

    let mut lines: Vec<OcrLine> = grouped.into_iter().map(OcrLine::from_spans).collect();
    lines.sort_by(|a, b| cmp_f32(a.center_y(), b.center_y()).then(cmp_f32(a.x_min, b.x_min)));
    (lines, fallback)
}

/// 替换翻译用：返回带坐标的 OCR 文本块（每个检测框一块，不合并同行多列）。
///
/// 表格/多栏布局若按视觉行合并，会把整行合成一个大框；逐块输出才能按单元格原位覆盖。
pub fn lines_from_results(results: &[oar_ocr::oarocr::OAROCRResult]) -> Vec<RapidOcrLine> {
    let (mut spans, _) = collect_ocr_spans(results);
    spans.sort_by(|a, b| cmp_f32(a.center_y(), b.center_y()).then(cmp_f32(a.x_min, b.x_min)));
    spans
        .iter()
        .enumerate()
        .map(|(index, span)| {
            let text = collapse_spaces(&span.text);
            RapidOcrLine {
                id: format!("s{index:04}"),
                text: text.clone(),
                points: span.points.clone(),
                x: span.x_min,
                y: span.y_min,
                width: (span.x_max - span.x_min).max(1.0),
                height: (span.y_max - span.y_min).max(1.0),
            }
        })
        .filter(|line| !line.text.is_empty())
        .collect()
}

/// 把所有 OCR 结果按阅读顺序拼成 Markdown-friendly 纯文本。
///
/// DBNet 返回的是一组文本框,不是排版树。这里做轻量几何后处理:
/// 1. 先按动态行高聚合同一视觉行,避免固定 30px 桶把大/小字号混在一起。
/// 2. 再按行距和右边界判断软换行/段落断开。
/// 3. 常见项目符号转成 Markdown list,让前端原文区和翻译模型都更容易理解结构。
fn join_text_regions(results: &[oar_ocr::oarocr::OAROCRResult]) -> String {
    let (lines, fallback) = collect_ocr_lines(results);

    if lines.is_empty() {
        return fallback.join("\n\n");
    }

    let mut out = format_ocr_lines(&lines);
    if !fallback.is_empty() {
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(&fallback.join("\n\n"));
    }
    out
}

fn span_belongs_to_line(line: &[OcrSpan], span: &OcrSpan, median_height: f32) -> bool {
    let x_min = line.iter().map(|s| s.x_min).fold(f32::INFINITY, f32::min);
    let y_min = line.iter().map(|s| s.y_min).fold(f32::INFINITY, f32::min);
    let x_max = line
        .iter()
        .map(|s| s.x_max)
        .fold(f32::NEG_INFINITY, f32::max);
    let y_max = line
        .iter()
        .map(|s| s.y_max)
        .fold(f32::NEG_INFINITY, f32::max);
    let line_height = (y_max - y_min).max(1.0);
    let span_height = span.height();
    let vertical_overlap = (y_max.min(span.y_max) - y_min.max(span.y_min)).max(0.0);
    let overlap_ratio = vertical_overlap / line_height.min(span_height);
    let center_delta = (((y_min + y_max) / 2.0) - span.center_y()).abs();
    if overlap_ratio < 0.45 && center_delta > line_height.min(span_height).max(median_height) * 0.55
    {
        return false;
    }

    let horizontal_gap = if span.x_min > x_max {
        span.x_min - x_max
    } else if x_min > span.x_max {
        x_min - span.x_max
    } else {
        0.0
    };
    horizontal_gap <= (median_height * 8.0).max(120.0)
}

fn format_ocr_lines(lines: &[OcrLine]) -> String {
    if lines.is_empty() {
        return String::new();
    }

    let median_height = median(lines.iter().map(OcrLine::height).collect()).unwrap_or(20.0);
    let doc_left = lines.iter().map(|l| l.x_min).fold(f32::INFINITY, f32::min);
    let doc_right = lines
        .iter()
        .map(|l| l.x_max)
        .fold(f32::NEG_INFINITY, f32::max);
    let doc_width = (doc_right - doc_left).max(1.0);

    let mut out = String::new();
    let mut prev_line: Option<&OcrLine> = None;
    let mut current_block_is_list = false;

    for line in lines {
        let text = normalize_line_text(&line.text);
        if text.is_empty() {
            continue;
        }
        let is_list = is_list_item(&text);

        if out.is_empty() {
            out.push_str(&text);
            current_block_is_list = is_list;
            prev_line = Some(line);
            continue;
        }

        let Some(prev) = prev_line else {
            out.push_str(&text);
            prev_line = Some(line);
            continue;
        };

        if should_merge_visual_line(
            prev,
            line,
            current_block_is_list,
            &out,
            &text,
            median_height,
            doc_left,
            doc_width,
        ) {
            append_inline(&mut out, &text);
        } else {
            if should_separate_blocks(
                prev,
                line,
                current_block_is_list,
                is_list,
                &out,
                median_height,
            ) {
                ensure_blank_line(&mut out);
            } else {
                ensure_line_break(&mut out);
            }
            out.push_str(&text);
            current_block_is_list = is_list;
        }
        prev_line = Some(line);
    }

    out
}

#[allow(clippy::too_many_arguments)]
fn should_merge_visual_line(
    prev: &OcrLine,
    current: &OcrLine,
    current_block_is_list: bool,
    out: &str,
    current_text: &str,
    median_height: f32,
    doc_left: f32,
    doc_width: f32,
) -> bool {
    if is_list_item(current_text) {
        return false;
    }

    let vertical_gap = current.y_min - prev.y_max;
    if vertical_gap < 0.0 || vertical_gap > median_height * 1.6 {
        return false;
    }

    if current_block_is_list && current.x_min > prev.x_min + median_height * 1.2 {
        return true;
    }

    let starts_near_previous = current.x_min <= prev.x_min + median_height * 1.2;
    let prev_reaches_line_end = prev.x_max >= doc_left + doc_width * 0.68;
    let previous_text = out.rsplit(['\n', '\r']).next().unwrap_or(out);
    let previous_looks_heading = looks_like_heading(previous_text, prev.height(), median_height);
    if previous_looks_heading {
        return false;
    }

    starts_near_previous && prev_reaches_line_end && !ends_with_sentence_break(previous_text)
}

fn should_separate_blocks(
    prev: &OcrLine,
    current: &OcrLine,
    previous_block_is_list: bool,
    current_is_list: bool,
    out: &str,
    median_height: f32,
) -> bool {
    let previous_text = out
        .rsplit(['\n', '\r'])
        .find(|s| !s.trim().is_empty())
        .unwrap_or(out);

    // Keep Markdown lists tight internally. Use a blank line only when entering or
    // leaving a list block so ReactMarkdown does not render every item as loose.
    if previous_block_is_list || current_is_list {
        return previous_block_is_list != current_is_list;
    }

    let vertical_gap = (current.y_min - prev.y_max).max(0.0);
    vertical_gap > median_height * 1.25
        || looks_like_heading(previous_text, prev.height(), median_height)
}

fn join_line_spans(spans: &[OcrSpan]) -> String {
    let mut out = String::new();
    let mut prev: Option<&OcrSpan> = None;
    for span in spans {
        let text = collapse_spaces(&span.text);
        if text.is_empty() {
            continue;
        }
        if let Some(prev_span) = prev {
            let gap = (span.x_min - prev_span.x_max).max(0.0);
            if should_insert_inline_space(&out, &text, gap, prev_span.height().max(span.height())) {
                out.push(' ');
            }
        }
        out.push_str(&text);
        prev = Some(span);
    }
    out
}

fn normalize_line_text(text: &str) -> String {
    let text = collapse_spaces(text);
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    if let Some(ordered) = normalize_ordered_list_item(trimmed) {
        return ordered;
    }

    let mut chars = trimmed.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    let rest = chars.as_str().trim_start();
    let marker = matches!(
        first,
        '•' | '·' | '●' | '○' | '◦' | '▪' | '▫' | '-' | '*' | '–' | '—'
    ) || ((first == 'O' || first == 'o' || first == '0')
        && !rest.is_empty()
        && rest
            .chars()
            .next()
            .is_some_and(|c| c.is_uppercase() || !c.is_ascii()));

    if marker && !rest.is_empty() {
        format!("- {rest}")
    } else {
        trimmed.to_string()
    }
}

fn normalize_ordered_list_item(text: &str) -> Option<String> {
    let mut digit_end = 0;
    let mut digit_count = 0;
    for (idx, c) in text.char_indices() {
        if c.is_ascii_digit() {
            digit_count += 1;
            digit_end = idx + c.len_utf8();
        } else {
            break;
        }
    }
    if digit_count == 0 || digit_count > 3 || digit_end >= text.len() {
        return None;
    }

    let marker_text = &text[digit_end..];
    let marker = marker_text.chars().next()?;
    if !matches!(marker, '.' | ')' | '）' | '、') {
        return None;
    }

    let after_marker = &marker_text[marker.len_utf8()..];
    let had_space = after_marker
        .chars()
        .next()
        .is_some_and(|c| c.is_whitespace());
    let rest = after_marker.trim_start();
    if rest.is_empty() {
        return None;
    }

    // Avoid turning version-like lines such as "1.24.4" into a list item.
    if !had_space && rest.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        return None;
    }

    Some(format!("{}. {}", &text[..digit_end], rest))
}

fn should_insert_inline_space(
    prev_text: &str,
    next_text: &str,
    gap: f32,
    line_height: f32,
) -> bool {
    if prev_text.is_empty() || next_text.is_empty() || gap <= 1.0 {
        return false;
    }
    let Some(prev) = prev_text.chars().last() else {
        return false;
    };
    let Some(next) = next_text.chars().next() else {
        return false;
    };
    if prev.is_whitespace()
        || next.is_whitespace()
        || matches!(
            next,
            ',' | '.'
                | ':'
                | ';'
                | ')'
                | ']'
                | '}'
                | '，'
                | '。'
                | '、'
                | '：'
                | '；'
                | '）'
                | '】'
        )
        || matches!(prev, '(' | '[' | '{' | '（' | '【')
    {
        return false;
    }
    gap > line_height * 0.15 || (is_ascii_word(prev) && is_ascii_word(next))
}

fn append_inline(out: &mut String, text: &str) {
    if out.ends_with('-') {
        out.pop();
    } else if out
        .chars()
        .last()
        .is_some_and(|c| !c.is_whitespace() && needs_space_before(text, c))
    {
        out.push(' ');
    }
    out.push_str(text);
}

fn needs_space_before(next_text: &str, previous: char) -> bool {
    let Some(next) = next_text.chars().next() else {
        return false;
    };
    !is_cjk(previous)
        && !is_cjk(next)
        && !matches!(
            next,
            ',' | '.'
                | ':'
                | ';'
                | ')'
                | ']'
                | '}'
                | '，'
                | '。'
                | '、'
                | '：'
                | '；'
                | '）'
                | '】'
        )
}

fn ensure_blank_line(out: &mut String) {
    if out.ends_with("\n\n") {
        return;
    }
    if out.ends_with('\n') {
        out.push('\n');
    } else {
        out.push_str("\n\n");
    }
}

fn ensure_line_break(out: &mut String) {
    if !out.ends_with('\n') {
        out.push('\n');
    }
}

fn is_list_item(text: &str) -> bool {
    is_markdown_bullet(text) || is_ordered_list_item(text)
}

fn is_markdown_bullet(text: &str) -> bool {
    text.trim_start().starts_with("- ")
}

fn is_ordered_list_item(text: &str) -> bool {
    let text = text.trim_start();
    let mut digit_end = 0;
    let mut digit_count = 0;
    for (idx, c) in text.char_indices() {
        if c.is_ascii_digit() {
            digit_count += 1;
            digit_end = idx + c.len_utf8();
        } else {
            break;
        }
    }
    if digit_count == 0 || digit_count > 3 || digit_end >= text.len() {
        return false;
    }
    text[digit_end..].starts_with(". ")
}

fn looks_like_heading(text: &str, height: f32, median_height: f32) -> bool {
    let text = text.trim();
    !text.is_empty()
        && text.chars().count() <= 80
        && height >= median_height * 1.2
        && !ends_with_sentence_break(text)
        && !is_list_item(text)
}

fn ends_with_sentence_break(text: &str) -> bool {
    text.trim_end().chars().last().is_some_and(|c| {
        matches!(
            c,
            '.' | '!' | '?' | ':' | ';' | '。' | '！' | '？' | '：' | '；'
        )
    })
}

fn collapse_spaces(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(crate) fn median(mut values: Vec<f32>) -> Option<f32> {
    values.retain(|v| v.is_finite());
    if values.is_empty() {
        return None;
    }
    values.sort_by(|a, b| cmp_f32(*a, *b));
    Some(values[values.len() / 2])
}

pub(crate) fn cmp_f32(a: f32, b: f32) -> std::cmp::Ordering {
    a.partial_cmp(&b).unwrap_or(std::cmp::Ordering::Equal)
}

fn is_ascii_word(c: char) -> bool {
    c.is_ascii_alphanumeric()
}

fn is_cjk(c: char) -> bool {
    matches!(
        c as u32,
        0x3400..=0x4dbf | 0x4e00..=0x9fff | 0xf900..=0xfaff | 0x3040..=0x30ff | 0xac00..=0xd7af
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_from_results_empty_input() {
        assert!(lines_from_results(&[]).is_empty());
    }
}

/// 真实端到端验证:真实下载 + 真实推理(PP-OCRv6 medium)。
///
/// 门控:`RAPIDOCR_E2E=1` 才跑,否则打印 skip 直接返回(不算失败,CI/常规跑测试不受影响)。
/// 模型缓存目录固定在系统临时目录下的一个子目录,重复运行不用每次重下 ~150MB。
#[cfg(all(test, target_os = "windows"))]
mod rapidocr_e2e {
    use super::*;

    #[tokio::test]
    async fn rapidocr_e2e_real_download_and_inference() {
        if std::env::var("RAPIDOCR_E2E").as_deref() != Ok("1") {
            eprintln!("[rapidocr-e2e] RAPIDOCR_E2E != 1, skipping real download+inference test");
            return;
        }

        // 持久缓存目录:重复跑不用每次重下模型。
        let model_dir = std::env::temp_dir().join("kivio-rapidocr-e2e-models");
        std::fs::create_dir_all(&model_dir).expect("mkdir model_dir");
        eprintln!("[rapidocr-e2e] model_dir = {}", model_dir.display());

        let client = RapidOcrClient::with_model_dir(model_dir.clone(), reqwest::Client::new());

        // 测试图:白底黑字中英混排,由仓库脚本(PowerShell + System.Drawing)生成。
        let image_path = std::env::temp_dir().join("kivio-rapidocr-e2e-test.png");
        generate_test_image(&image_path);
        assert!(
            image_path.is_file(),
            "test image should exist at {}",
            image_path.display()
        );

        // 1) 下载(共享 dylib + PP-OCRv6 模型)。
        let t0 = std::time::Instant::now();
        let install = client.install().await;
        eprintln!(
            "[rapidocr-e2e] install: success={} message={:?} elapsed={:?}",
            install.success,
            install.message,
            t0.elapsed()
        );
        assert!(install.success, "install failed: {}", install.message);

        // 2) status:应就绪。
        let status = client.status();
        eprintln!("[rapidocr-e2e] status = {status:?}");
        assert!(status.available, "models should be available after install");

        // 3) 真实推理。
        let t2 = std::time::Instant::now();
        let text = client
            .ocr_image(&image_path)
            .await
            .expect("ocr_image should succeed");
        eprintln!(
            "[rapidocr-e2e] OCR result (elapsed={:?}):\n{text}",
            t2.elapsed()
        );
        assert!(!text.is_empty(), "OCR text should not be empty");
        assert!(
            text.contains("Kivio"),
            "OCR text should contain 'Kivio', got: {text}"
        );
        assert!(
            text.contains("测试") || text.contains("识别"),
            "OCR text should contain a Chinese keyword ('测试'/'识别'), got: {text}"
        );

        // 4) 带坐标的行级 OCR(替换翻译用),pipeline 复用,断言坐标合理。
        let lines = client
            .ocr_image_lines(&image_path)
            .await
            .expect("ocr_image_lines should succeed");
        eprintln!("[rapidocr-e2e] ocr_image_lines: {lines:?}");
        assert!(
            !lines.is_empty(),
            "ocr_image_lines should return at least one line"
        );
        for line in &lines {
            assert!(
                line.width > 0.0 && line.height > 0.0,
                "line bbox should be positive: {line:?}"
            );
        }
    }

    /// 用仓库脚本 `scripts/gen-rapidocr-e2e-image.ps1`(PowerShell + System.Drawing)生成测试图。
    fn generate_test_image(out: &Path) {
        let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("scripts")
            .join("gen-rapidocr-e2e-image.ps1");
        let status = std::process::Command::new("powershell")
            .arg("-NoProfile")
            .arg("-ExecutionPolicy")
            .arg("Bypass")
            .arg("-File")
            .arg(&script)
            .arg("-OutputPath")
            .arg(out)
            .status()
            .expect("failed to spawn powershell to generate test image");
        assert!(status.success(), "gen-rapidocr-e2e-image.ps1 failed");
    }
}
