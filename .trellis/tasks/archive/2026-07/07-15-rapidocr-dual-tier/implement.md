# Implement — RapidOCR 双档位

按顺序做;每步后编译。参照命令:`git show 3f51ec3^:<file>` 取原实现,在当前文件上重新接线(不 `git revert`)。

## 步骤

1. **offline_models.rs — 模型清单分档**
   - 恢复 `OcrModelTier { Standard, High }`(+ `as_str` / 未知→Standard)。
   - 恢复 `STANDARD_DET/REC/KEYS`(v5 mobile,根目录,URL/SHA 见 design)与 `HIGH_DET/REC/KEYS`(v6,`high/`,即现在的 OCR_DET/REC/KEYS 改名回 HIGH_*)。
   - `rapidocr_pack(tier)` / `replace_translation_pack(tier)` 按档返回文件集;`rapidocr_ready(tier)`、`install_rapidocr(tier)`、`replace_translation_status(tier)` 带档位。
   - 校验:`cargo check --lib`。

2. **rapidocr.rs — 双 pipeline + 分档阈值(保留计时)**
   - 双 `OnceCell` + `pipeline_for(tier)`(路径 + 阈值分档:Standard 默认阈值 / High 0.2/0.45/1.4)。
   - `ocr_image` / `ocr_image_lines` 带 tier;**保留**内部三段计时插桩。
   - `RapidOcrStatus { standard_available, high_available }`;`warmup(tier)`。
   - 校验:`cargo check --lib`。

3. **settings.rs — 档位字段**
   - `ScreenshotTranslationConfig.rapid_ocr_tier`(default `"standard"`);`DocumentProcessingConfig.rapid_ocr_tier`(default `"high"`)。
   - `sanitize_settings` 归一非法值。
   - 校验:`cargo check --lib`。

4. **commands.rs / lens_commands.rs / process.rs — 接线**
   - 命令带 tier(status 返回双 flag;install/pack_status/pack_install 带 tier)。
   - `lens_replace_translate`:解析截图翻译 tier → 校验该档 → `ocr_image_lines(tier)`;入口预热改 `warmup(tier)`。
   - KB `process.rs`:图片 OCR 用 document_processing tier。
   - 校验:`cargo check --lib`。

5. **前端 — 类型 + 设置页 UI**
   - `api/tauri.ts`:`RapidOcrStatus` 双 flag;install/status API 带 tier。
   - `ScreenshotTranslationSettings.tsx` / `DocumentProcessingPanel.tsx`:档位选择器 + 每档下载/就绪;`SettingsShell.tsx` 接线;`i18n.ts` 标准/高。
   - 校验:`npm run typecheck` + `npm run lint`。

6. **验证(真机)**
   - `npm run dev`,设置里选「标准」→ 触发替换翻译下载 v5(~20MB)→ 跑一次,看 `[replace-timing]` 的 `predict` 段是否从 ~4s 降到 ~1–1.5s。
   - 切「高」→ 确认仍用已装 v6、`high_available=true` 不重下。
   - 截图翻译 + 知识库图片入库各跑一次,确认按各自档位走。

7. **收尾**
   - 计时插桩(`[replace-timing]` / `__lap`)本是临时诊断:验证达标后决定删或留(评审定);预热保留。
   - 更新 spec `.trellis/spec/frontend/replace-translation.md`(双档位)。

## 校验命令
- Rust:`cargo check --manifest-path src-tauri/Cargo.toml --lib`
- 前端:`npm run typecheck` && `npm run lint`
- Rust 测试基线:`cargo test --manifest-path src-tauri/Cargo.toml --lib`(对比既有 ~14 个 env/locale 预存失败,非本任务回归)

## 回滚点
- 每步独立编译;若某步搞砸,`git checkout -- <file>` 回该文件(注意别误删本次计时/预热或其他 186 个在改文件)。
- 模型下载失败不影响代码路径(走现有 missing→下载提示)。

## 冲突 / 注意
- 只与本次未提交的计时/预热改动交叠(在 rapidocr.rs / lens_commands.rs);逐文件手动融合,别覆盖掉计时/预热。
- 不要把当前工作区那个半成品 regions→groups/slots 重构掺进来;typecheck 已有的红项先甄别是否本任务引入。
