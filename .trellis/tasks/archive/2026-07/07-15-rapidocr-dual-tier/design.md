# Design — RapidOCR 双档位

## 恢复参照
被移除的完整双档实现在 `3f51ec3^`(命名再看 `781b4e9^`)。用 `git show 3f51ec3^:<file>` 逐文件取原实现作参照,在当前 HEAD + 本次未提交的计时/预热改动之上重新接线。**不用 `git revert`**——会和 rapidocr.rs / lens_commands.rs 里我加的计时+预热冲突。`781b4e9..HEAD` 已确认没有其他提交碰过这些核心 Rust 文件,冲突面只有本次未提交改动。

## 数据模型 / 契约

### `OcrModelTier`(offline_models.rs)
```
pub enum OcrModelTier { Standard, High }   // as_str: "standard" / "high"; 未知值 → Standard
```

### 模型清单(offline_models.rs)—— URL/SHA 从 `3f51ec3^` 原样取回
- **标准(v5 mobile)** → 存模型目录**根**:
  - `det.onnx` ← GreatV/oar-ocr v0.3.0 `pp-ocrv5_mobile_det.onnx`, sha `1eb7b4f7…`
  - `rec.onnx` ← `pp-ocrv5_mobile_rec.onnx`, sha `243a0f06…`
  - `keys.txt` ← `ppocrv5_dict.txt`, sha `d1979e9f…`
- **高(v6 medium,现状)** → 存 `high/` 子目录:`high/det.onnx`(ModelScope `pp-ocrv6_medium_det.onnx` sha `eb13b44b…`)、`high/rec.onnx`、`high/keys.txt`。
- `replace_translation_pack()` / `rapidocr_pack()` 改为**按档位返回文件集**;`rapidocr_ready(tier)` / `install_rapidocr(tier)` / `replace_translation_status(tier?)` 带档位。

### `RapidOcrClient`(rapidocr.rs)
- 双 `OnceCell`:`standard_pipeline` + `high_pipeline`;`pipeline_for(tier)` 按档取路径 + 阈值:
  - Standard → 根 det/rec/keys,**默认阈值**(不 override,即 0.3/0.6/1.5)。
  - High → `high/` det/rec/keys,阈值 `score 0.2 / box 0.45 / unclip 1.4`。
- `ocr_image_lines` / `ocr_image` 增加 `tier` 入参(或读配置解析出 tier),内部走 `pipeline_for(tier)`。
- `RapidOcrStatus { standard_available, high_available }`。
- `warmup(tier)`:预热指定档(替换本次已加的无参 warmup)。

### 设置(settings.rs)
- `ScreenshotTranslationConfig.rapid_ocr_tier: String`(serde default `"standard"`)。
- `DocumentProcessingConfig.rapid_ocr_tier: String`(serde default `"high"` —— 入库要精度)。
- `sanitize_settings`:非法值归一到各自默认(参照 `3f51ec3` 删掉的那段 `!= "standard"` 逻辑)。

### 命令(commands.rs / lens_commands.rs)
- `rapidocr_status()` → 返回 `{ standard_available, high_available }`。
- `rapidocr_install(tier)` / `replace_translation_pack_status(tier)` / `replace_translation_pack_install(tier)` 带档位参数。
- `lens_replace_translate`:从 `settings.screenshot_translation.rapid_ocr_tier` 解析 tier → 就绪校验该档 → `ocr_image_lines(tier)`;入口预热同一 tier。
- KB `process.rs`:图片 OCR 用 `document_processing.rapid_ocr_tier`。

### 前端(api/tauri.ts + 设置页)
- `RapidOcrStatus` 类型 → `{ standardAvailable, highAvailable }`;install/status API 带 tier。
- `ScreenshotTranslationSettings.tsx` / `DocumentProcessingPanel.tsx`:恢复档位选择器(标准/高),每档显示就绪/下载。
- `i18n.ts`:`标准` / `高` + 说明(标准=快/小,高=精度/大)中英。
- `SettingsShell.tsx`:接线(参照旧 diff)。

## 与本次已加改动的整合
- **计时插桩**(rapidocr.rs `ocr_image_lines` 内三段计时、lens_commands.rs `__lap`):保留。
- **预热**(lens_commands.rs `mode=="replace"` spawn):改为 `rapidocr.warmup(tier)`,tier 取自截图翻译配置。

## 兼容 / 迁移
- `high/` 子目录保持,现有 v6 安装 → 「高」档直接 ready,不重下。
- 默认档翻转到「标准」→ v6-only 用户首用替换翻译触发 `rapidocr_models_missing`(standard 档),前端按现有路径提示下载 ~20MB。确认这条链路仍走通、不 panic。

## 取舍
- 不引入运行时"自动选档";用户显式选。默认值负责多数场景(截图=快,入库=准)。
- 不动 `high/` 布局与 v6 阈值,最大化复用现有安装与已验证参数。
