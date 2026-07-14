# 笔画级擦除实施计划

## Dependencies

- geometry-core 提供稳定 leaves 与 source geometry。
- semantic-routing 可提供图标/控件/表格保护边界；无语义结果时仍需独立工作。
- visual-benchmark 先提供 mask 与结构保护指标。

## Steps

1. 固化 `EraseMaskSource`、protection mask 和 repair router 接口。
2. 把现有颜色聚类 mask 重构为可重复基线，补足随机 tie、欠框和结构线测试。
3. 调研并原型化 1–2 个笔画分割候选，转换 ONNX 并测量体积/延迟。
4. 实现局部 patch 批处理和 probability → binary mask refinement。
5. 接入语义/布局 protection masks，保护头像、图标、边框、分隔线和表格。
6. 重构 UI/table/gradient/photo repair router，保证 mask 外逐像素不变。
7. 统一离线包 manifest、下载校验、状态和进度。
8. 完成跨平台真实性能与视觉门禁。

## Validation

```bash
cargo test --manifest-path src-tauri/Cargo.toml replace_translation::mask
cargo test --manifest-path src-tauri/Cargo.toml inpainting
cargo test --manifest-path src-tauri/Cargo.toml offline_models
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
```

真实模型检查：重复运行一致性、mask 外像素一致、UI/文档本地热路径总预算、Windows/macOS ONNX Runtime 兼容。

## Rollback

笔画模型可以撤下到确定性 stroke baseline，但不能退回整行矩形 mask。MI-GAN 可独立撤下到受保护的局部修复，不影响 geometry/routing。

## 2026-07-14 Progress (macOS-verifiable, no-model slice)

只交付了两个确定性、无模型、可在 macOS 本地单测验证的构件；本次**没有**触碰任务的核心目标。

### 现在有单测证明的（unit-test-verified）

- [x] AC-E3 mask 外逐像素不变（`src-tauri/src/inpainting.rs`）：把 `run_inpainting` 内联的合成逻辑抽成纯函数 `blend_inpaint_output(source, binary_mask, model_output_nchw)`，行为逐字节保持不变；新增无 ORT / 无模型 / 无网络的单测：mask==0 处逐字节等于 source、mask!=0 处等于解码后的伪模型输出、错误长度返回 `OutputInvalid`。`cargo test ... inpainting` = 6 passed / 1 ignored（`inpainting_real_e2e` 仍需真实模型，未跑）。
- [x] AC-E2 确定性结构保护脚手架（`src-tauri/src/replace_translation/mask.rs`）：新增 `build_rule_protection` + `apply_protection`，只检测长的近恒定颜色横/竖细线（表格线 / 分隔线 / 单元格边框），保守偏向 false-negative，绝不吞字形。合成图单测断言细线像素被保护、文字块仍可擦除。这些函数**尚未接入运行时**（`lens_commands.rs` 未改），是供后续接线用的受测构件。

### 明确未做、仍是缺口的（NOT done）

- [ ] **任务标题目标：文字笔画分割模型**（stroke/glyph segmentation ONNX）完全未做。没有引入任何模型、ONNX 文件、依赖或下载；擦除仍是原有颜色聚类回退，没有 stroke-probability 路径。
- [ ] **图标 / 头像 / 圆角控件保护**未做——`build_rule_protection` 只处理细直线，图标/头像框需要 semantic-routing 的语义边界（代码内已留 `// ponytail:` 延迟注释）。
- [ ] **MI-GAN 真实模型性能与体积预算**（AC-E5：1 秒热路径、离线包上限、Apple M 系列实测）未验证——本次只证明了合成阶段的像素不变式，未跑真实 ORT。
- [ ] **照片路径实机 + macOS/Windows 双平台视觉 QA**未做——AC-E1/E2 的视觉验收、AC-E4 真实修复确定性均须实机截图门禁，归属 visual-benchmark 最终关闭。
- [ ] E5（膨胀/细化策略：欠框、抗锯齿、描边、阴影、旋转）、E6（背景簇/mask/修复端到端确定性）、E7（统一离线包 manifest/校验/进度）未触及。

结论：本次为窗口化、可回归的最小切片，**不能据此归档整个 stroke-erasure 任务**。

