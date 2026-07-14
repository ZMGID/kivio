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
