# 替换翻译 V2 总体实施计划

## Dependency Order

1. 先启动 `07-14-replace-translation-visual-benchmark`，建立 fixture schema、已知回归基线和阶段产物格式。
2. 实施 `07-14-replace-translation-geometry-core`，作为所有后续任务的契约与坐标基础。
3. 在 geometry-core 稳定后并行推进 `07-14-replace-translation-semantic-routing` 与 `07-14-replace-translation-stroke-erasure`。
4. 三个基础任务达到各自门禁后实施 `07-14-replace-translation-scene-rendering`。
5. visual-benchmark 汇总 Windows/macOS 和所有场景分桶结果，最后关闭父任务。

## Phase 0: Planning Gate

- [x] 用户确认 Windows 与 macOS 同期交付。
- [x] 用户确认直接使用新架构，不保留长期双管线或灰度开关。
- [x] 用户确认完整翻译优先，不进行简洁化或摘要。
- [x] 用户确认 Standard 250 MB、High 450 MB、UI/文档 1 秒、照片 3 秒预算。
- [x] 父任务和五个子任务均具备 `prd.md`、`design.md`、`implement.md`。
- [ ] 运行 Trellis artifact validation，并记录当前 active task。
- [ ] 加载 `trellis-before-dev` 的 workflow/spec 上下文。

## Phase 1: Visual Benchmark Foundation

- [ ] 建立版本化 fixture 目录、schema、隐私规则和结果目录。
- [ ] 将菜单/列表、表格、段落整体上移、保留高亮、边框异常、照片残影等已知问题转为可重复 fixture 或等价合成样例。
- [ ] 建立固定 OCR/翻译输入的离线 runner，输出 leaves、groups、slots、mask、cleaned image、final image 和 metrics。
- [ ] 固化 geometry、first-anchor drift、跨列/跨区、保护像素、旧字残影、译文完整性和性能指标。

验证以子任务 `07-14-replace-translation-visual-benchmark/implement.md` 为准。

## Phase 2: Geometry Core

- [ ] 定义并接入 `OcrLeaf`、`TranslationGroup`、`RenderSlot`、`EraseMask`。
- [ ] 统一自然像素坐标、CapturedFrame 映射和 macOS/Windows 缩放元数据。
- [ ] 实现稳定 ID 翻译映射与局部回退。
- [ ] 实现保守行级默认路径、段落顶部锚定和结构内完整文本适配。
- [ ] 一次性升级 V2 事件协议和前端自然尺寸渲染。
- [ ] 删除达到门禁后的旧多职责区域与旧 Canvas 矩形绘制路径。

验证以子任务 `07-14-replace-translation-geometry-core/implement.md` 为准。

## Phase 3A: Semantic Routing

- [ ] 实现共享 `SemanticGeometryProvider` / `SemanticElement`。
- [ ] macOS 接入 Accessibility；Windows 接入 UI Automation。
- [ ] 统一角色、层级、坐标、DPI/Retina、多显示器、超时和错误语义。
- [ ] 实现场景路由和 OCR 无感回退。
- [ ] 评估轻量文档布局证据，未通过预算或质量门禁时不接入。

验证以子任务 `07-14-replace-translation-semantic-routing/implement.md` 为准。

## Phase 3B: Stroke Erasure

- [ ] 建立笔画级 mask 基线并评估轻量分割模型。
- [ ] 实现抗锯齿、描边、阴影、旋转文字的受限膨胀/细化。
- [ ] 实现 UI/表格结构保护和确定性快速修复。
- [ ] 照片复杂背景只在 mask 内调用修复器，保证 mask 外逐像素不变。
- [ ] 将模型、延迟和峰值内存纳入统一资源门禁。

验证以子任务 `07-14-replace-translation-stroke-erasure/implement.md` 为准。

## Phase 4: Photo Scene Rendering

- [ ] 用独立实验评估背景修复、确定性目标字形、风格编码和仿射融合候选。
- [ ] 覆盖中英、长度变化、旋转、透视、颜色、阴影和纹理样例。
- [ ] 只有满足许可证、总包体、3 秒热路径、字体覆盖和视觉门禁的方案才接入。
- [ ] 未达标时由 V2 geometry-core 系统字体路径承担 photo route。

验证以子任务 `07-14-replace-translation-scene-rendering/implement.md` 为准。

## Phase 5: Cross-Platform Release Gate

- [ ] 运行 Rust format、Clippy、测试，前端 lint、typecheck、测试和 build。
- [ ] macOS 验证 Accessibility、Retina、刘海/安全区、多显示器、模型冷/热路径和取消行为。
- [ ] Windows 验证 UI Automation、100/125/150/200% DPI、多显示器、runtime DLL、下载与模型推理。
- [ ] 按 UI、document、table、photo、unknown 和 platform 分桶生成报告；禁止用平均分掩盖失败场景。
- [ ] 确认普通截图翻译、选中文本翻译和知识库 RapidOCR 未回归。
- [ ] 确认生产代码仅剩 V2 单管线后归档五个子任务及父任务。

## Common Validation Commands

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
npm run lint
npm run typecheck
npm test
npm run build:ui
```

子任务可先运行更小的专项命令，但最终门禁必须执行完整命令并区分本任务回归与工作区既有问题。

## Risky Files

- `src-tauri/src/lens_commands.rs`：主流程、并行阶段和事件协议。
- `src-tauri/src/rapidocr.rs`：OCR 几何、稳定 ID 和共享模型生命周期。
- `src-tauri/src/prompts.rs`：固定翻译 JSON 契约。
- `src-tauri/src/replace_translation/`：geometry、routing、mask、repair 和 benchmark 新模块。
- `src/lens/ReplaceTranslateOverlay.tsx`、`src/lens/replaceTextLayout.ts`、`src/Lens.tsx`：自然尺寸绘制和覆盖层状态机。
- macOS/Windows 平台模块：Accessibility、UI Automation、DPI 和屏幕坐标。

## Rollback Points

- 每个阶段必须保留可运行的 V2 保守 OCR/系统字体路径。
- 原生语义 API 或候选模型失败只回退 V2 内部能力，不恢复旧生产管线。
- 达不到照片门禁时不打包对应模型。
- 工作区已有大量用户改动；实施前逐文件检查重叠，只修改当前子任务范围。
