# 替换翻译视觉基准实施计划

## Dependencies

- 本任务最先开始，先提供 fixture schema 与核心几何/擦除基线。
- 随 geometry、routing、erasure、scene 子任务逐步扩展产物和门禁。
- 最终归档晚于其他所有子任务的质量检查。

## Steps

1. 建立 fixture 目录、`case.json` schema、结果目录和隐私规则。
2. 把已知文档、仓库列表、表格、代码徽章和照片失败样例转为本地/合成夹具。
3. 实现离线 runner：固定 OCR/翻译输入时可只跑布局、mask 和 Canvas。
4. 输出 geometry/mask/cleaned/final 阶段文件和 contact sheet。
5. 实现几何、重叠、保护像素、残影、完整性和性能指标。
6. 建立 macOS/Windows 平台 fixture 执行清单和结果合并格式。
7. 为每个子任务固化初始阈值与 required buckets。
8. 集成到本地质量命令；适合 CI 的无模型测试进入 CI，真实模型/实机测试作为发布前门禁。
9. 最终生成一份场景分桶报告，所有必需桶通过后允许父任务收尾。

## Validation

```bash
npm test -- src/lens
cargo test --manifest-path src-tauri/Cargo.toml replace_translation
npm run typecheck
npm run build:ui
```

新增命令目标：

```bash
npm run test:replace-visual
cargo test --manifest-path src-tauri/Cargo.toml replace_translation::visual_fixtures
```

## Rollback

基准工具只读源图并写结果目录，不改变生产状态。指标不稳定时先降级为报告项，不能删除原始夹具或以人工“看起来可以”替代。

## 2026-07-14 Progress

- [x] 新增 V1 fixture schema 与 5 组确定性合成夹具：菜单、文档段落、表格、代码徽章、渐变照片。
- [x] macOS/Windows 平台、Retina/DPI 和场景标签写入每个 `case.json`。
- [x] 新增几何锚点、IoU、跨区/跨列、保护像素、mask 外像素和译文完整性指标。
- [x] 新增按场景分桶的发布门禁，缺少 required bucket 或照片桶失败时不能被平均分掩盖。
- [x] 新增 `npm run fixtures:replace-visual` 与 `npm run test:replace-visual`。
- [ ] 后续子任务接入真实 leaves/groups/slots/mask/cleaned/final 阶段产物与 contact sheet。
- [ ] 完成 macOS/Windows 实机结果合并和最终发布报告。

## 2026-07-14 Progress（真实管线门禁补齐）

审计发现：此前基准只把**手工合成数字**喂给指标函数，没有任何夹具的真实图像/OCR 走过真实布局/擦除管线，也从不校验 `expected_geometry.json` 是否与绘制一致，因此无法捕获 `layout.rs`/`mask.rs` 的真实回归（是自洽的重言式）。本次把它变成真正的回归门禁：

- [x] 生成器（`scripts/generate-replace-visual-fixtures.mjs`）现在为每个夹具额外产出确定性 `leaves.json`（固定的地面真值 OCR 输入：`{ id, text, quad(自然像素), readingOrder }`），并**从同一套绘制坐标重生成 `expected_geometry.json` 的锚点**（每条绘制行的左上角）。leaves 与 expected 因此**由构造保证互相一致**，且 anchor 是地面真值、绝不取自管线本身。`case.json` 新增 `"leaves"` 字段；`replaceVisualBenchmark.ts` 增加 `leaves` 解析/类型（`ReplaceVisualLeaf`）。`npm run fixtures:replace-visual` 已重生成，`npm run test:replace-visual`（含 `--check`）干净。
- [x] 新增 Rust 集成测试 `src-tauri/src/replace_translation/visual_fixtures.rs`（`#[cfg(test)] mod visual_fixtures;`）。它对 `tests/fixtures/replace-translation/v1/*/` **每个夹具**：由 `leaves.json` 构造 `Vec<RapidOcrLine>`（quad→points+bounds）、加载 `source.png`，跑**真实** `filter_replaceable_spans` + `build_replace_geometry` + `analyze_text_regions`（只读消费公共 API），然后断言：
  - render-slot 锚点与 `expected_geometry.json` 锚点误差在阈值内（UI/文档/表格 2px、照片 4px）——这是「整体上移/跨行合并」门禁，exact_line/UI 场景每行保持自身左上角锚点、首行零漂移；
  - slot 数量不塌缩为 1（菜单 3、表格 4 保持），无跨列合并、无丢行/造行；
  - 擦除 mask 覆盖字形像素但不铺满整个多边形矩形（覆盖率 2%–98% 的合理性）。
  - 夹具目录经 `CARGO_MANIFEST_DIR/../tests/...` 稳健定位；**完全确定性，无 OCR 模型、无 ORT、无网络**。
- [x] `replaceVisualBenchmark.ts` + `.test.ts` 补上审计标记缺失的两个指标：(a) 原文残影/ghost 输入——`computeReplacePixelMetrics` 新增可选 `originalTextMask` 与 `originalTextPixelCount`/`residualTextPixelCount`/`residualTextRatio` 字段及 `maxResidualTextRatio` 门禁；(b) 段落首行顶锚漂移回归用例——多行段落整体下移时 `maxFirstAnchorDrift`/`maxTopError` 均能捕获、忠实渲染零漂移通过。
- [x] 门禁在当前 `layout.rs` 上**真实通过**：把锚点阈值收紧到 0.001px 仍全绿，证明真实管线锚点与地面真值**逐一精确相等（漂移 0.0）**，非重言式（expected 独立于管线）。未放宽阈值凑绿。
- [x] 验证全绿：`cargo test … replace_translation::visual_fixtures`（1 passed）、`npx vitest run …replaceVisualBenchmark.test.ts`（10 passed）、`npm run typecheck` 干净、`cargo fmt … --check` 干净、专项 ESLint exit 0。
- 说明：`implement.md` Validation 里的模块名已从计划占位 `replace_translation_visual_fixtures` 校正为真实路径 `replace_translation::visual_fixtures`。

### 仍未完成（诚实缺口）

- [ ] macOS/Windows 实机截图运行与人工视觉目检（AC-V5 双平台门禁、AC-V1/V2 的目视记录）——本会话为无模型确定性 CI 级测试，未做实机。
- [ ] 真实 OCR 模型驱动的 leaves（现为生成器地面真值，非 RapidOCR/系统 OCR 实测输出）。
- [ ] MI-GAN/照片风格的真实重绘输出与其独立首版门禁（AC-V7）；本任务照片夹具只验证几何/擦除锚点，不背书照片重绘质量。
- [ ] contact sheet / before-after 视觉产物输出（本次范围外）。
- [ ] CI yaml 接线（把无模型测试正式纳入 CI 工作流）。
- 依设计，本任务须**最后归档**：AC-G1/G4/G6 等的验收侧与 macOS/Windows 实机视觉门禁绑定，须双平台人工目检并合并结果后，由本任务最终门禁关闭父任务。
