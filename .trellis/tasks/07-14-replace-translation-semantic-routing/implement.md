# 跨平台语义几何实施计划

## Dependencies

- 必须在 geometry-core 的 V2 contracts 稳定后接入。
- visual-benchmark 先提供 macOS/Windows 对等 fixture schema。

## Steps

1. 定义共享 `SemanticElement`、provider trait、错误与坐标转换测试。
2. macOS 实现 AX provider：权限、目标窗口/元素遍历、role/value/frame、超时与裁剪。
3. Windows 实现 UI Automation provider：COM 生命周期、ControlView/RawView、BoundingRectangle、DPI 转换与超时。
4. 建立 OCR leaf 与 semantic element 匹配器，输出匹配证据和置信度。
5. 实现场景路由与 Unknown 保守回退。
6. 评估/转换轻量文档布局模型；记录模型体积、CPU 延迟和各场景准确率。
7. 把语义硬边界接入 translation group/render slot 生成，不改变 erase mask leaf 归属。
8. 完成 Windows/macOS 对等实机和自动测试。

## Validation

```bash
cargo test --manifest-path src-tauri/Cargo.toml semantic_geometry
cargo test --manifest-path src-tauri/Cargo.toml replace_translation::routing
cargo test --manifest-path src-tauri/Cargo.toml replace_translation::layout
npm run typecheck
```

实机门禁：

- macOS：原生应用、Chromium/Safari、无 AX Canvas、多显示器 Retina。
- Windows：Win32/WPF/Chromium、自绘 Canvas、100/125/150/200% DPI、多显示器。

## Rollback

任一 provider 失败只回退 V2 exact_line OCR，不回退旧聚合管线。布局模型可独立撤下，不影响原生语义 provider。
