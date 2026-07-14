# 替换翻译几何核心实施计划

## Dependencies

- 前置：`07-14-replace-translation-visual-benchmark` 先建立核心几何夹具和指标骨架。
- 后续：semantic-routing、stroke-erasure、scene-rendering 均依赖本任务新契约。

## Steps

1. 建立 V2 类型与纯函数测试：`OcrLeaf`、`TranslationGroup`、`RenderSlot`、`ReplaceRenderPayloadV2`。
2. 把现有 `build_layout_regions` 拆成 translation grouping 与 render slot generation。
3. 实现 `exact_line`、`paragraph_flow`、`cell_flow` 三种 slot 策略和完整译文分配器。
4. 升级稳定 ID 翻译请求/响应，使翻译只绑定 group，不绑定绘制矩形。
5. 升级 `lens-replace-stream` 事件与 `src/api/tauri.ts` 解码。
6. 重写 Canvas 消费 slots；paragraph 顶部锚定，line/cell 使用显式垂直策略。
7. 删除 `LensReplaceRegion` 和旧单体 `ReplaceLayoutRegion` 生产路径。
8. 用菜单、段落、表格、长译文和 ID 异常夹具完成回归。

## Validation

```bash
cargo test --manifest-path src-tauri/Cargo.toml replace_translation::layout
cargo test --manifest-path src-tauri/Cargo.toml prompts
cargo test --manifest-path src-tauri/Cargo.toml lens_replace_translate
npm test -- src/lens/replaceTextLayout.test.ts src/lens/ReplaceTranslateOverlay.test.tsx
npm run typecheck
npm run build:ui
```

## Rollback Points

- V2 类型完成但未接线。
- 后端同时生成 V1/V2 仅用于测试对比。
- 前端切到 V2 后删除 V1；删除后不重新引入运行时开关。

## 2026-07-14 Progress

- [x] 建立 `OcrLeaf`、`TranslationGroup`、`RenderSlot` 与独立 `EraseMask` 契约。
- [x] 后端生产事件一次性升级到 V2 `groups + slots`，删除旧 `ReplaceLayoutRegion` 生产类型。
- [x] 默认保守路径改为每个 OCR 视觉行独立 group + `exact_line` slot；整页 groups 仍在一次请求中提供翻译上下文。
- [x] 前端事件边界集中校验版本、ID、group 引用、bounds 与枚举，拒绝旧 regions-only 载荷。
- [x] Canvas 的普通与安全缩放路径都从 OCR anchor 绘制；不再按译文高度重新垂直居中。
- [x] 菜单整体上移、启发式跨行重分配、未知 group、旧协议和跨列指标已有回归测试。
- [x] 替换翻译专项 Rust 测试、前端专项测试、typecheck、专项 ESLint 和 UI build 通过。
- [ ] macOS/Windows 实机截图视觉验收仍由 visual-benchmark 最终门禁完成。

### 2026-07-14 收尾（审计后补齐）

- [x] 修复对抗核验发现的 AC-G5 缺口：`ReplaceTranslateOverlay.test.tsx` 原来"执行但未断言" Canvas 自然尺寸，新增专项测试（CSS 320×180 / 自然 640×360）断言 backing size==自然像素、CSS style==逻辑尺寸，真正锁死 AC-G5 第三条与 AC-G10。前端替换测试 4/4→22/22 绿。
- [x] `npm run typecheck` 干净通过（LSP 曾报 stale 的 groups 属性错误，tsc 为准，无误）。
- [x] 替换翻译前端文件专项 ESLint exit 0。全量 `npm run lint` 唯一失败在 `src/chat/conversationExport.ts`（no-control-regex，提交 69609c6 的既有问题，与本任务无关，未改动）。
- [x] `build:ui` 未重跑：本次仅改测试文件，prod bundle 未变，上一轮已 build 绿。
- 结论：geometry-core **代码级交付物全部完成且全绿**；6 条代码级 AC 已有单测覆盖。任务**不能在本会话归档**——AC-G1/G4/G6 的验收侧与 macOS/Windows 实机视觉门禁绑定 visual-benchmark，须双平台人工目检后由其最终门禁关闭。
