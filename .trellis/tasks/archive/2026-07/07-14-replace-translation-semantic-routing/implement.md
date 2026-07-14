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
cargo test --manifest-path src-tauri/Cargo.toml replace_translation::semantic
cargo test --manifest-path src-tauri/Cargo.toml replace_translation::routing
cargo test --manifest-path src-tauri/Cargo.toml replace_translation::layout
npm run typecheck
```

实机门禁：

- macOS：原生应用、Chromium/Safari、无 AX Canvas、多显示器 Retina。
- Windows：Win32/WPF/Chromium、自绘 Canvas、100/125/150/200% DPI、多显示器。

## Rollback

任一 provider 失败只回退 V2 exact_line OCR，不回退旧聚合管线。布局模型可独立撤下，不影响原生语义 provider。

## 2026-07-14 Progress (foundation slice)

范围：只实现「macOS 可验证 + 纯新模块」切片（计划 Step 1、Step 2 的 macOS 部分、Step 4、Step 5）。新增两个文件，仅通过 `mod.rs` 两行 `pub mod` 接入，未接入实时管线。

新增/改动文件：

- `src-tauri/src/replace_translation/semantic.rs`（新）：`SemanticRole` / `SemanticSource` / `SemanticError` / `SemanticElement` / `SemanticQueryContext` / `SemanticGeometryProvider` 契约；纯坐标转换 `screen_to_capture`；平台中立 `NoSemanticProvider`（R13 回退契约）；macOS `MacosAxProvider`（`#[cfg(target_os = "macos")]`，复用 shortcuts.rs 的 ApplicationServices FFI 风格）；纯匹配器 `match_leaves_to_elements`。
- `src-tauri/src/replace_translation/routing.rs`（新）：`SceneKind`、纯 `classify_scene` -> `SceneDecision`（reason + evidence）、低置信度统一降级 Unknown 的门禁、`render_flow_for_scene`（Unknown -> `RenderFlow::ExactLine`，只读消费 layout.rs 公有枚举）。
- `src-tauri/src/replace_translation/mod.rs`：仅新增 `pub mod routing;` 与 `pub mod semantic;` 两行。

### 已单测验证（本机 macOS/arm 确定性逻辑）

- 坐标转换 `screen_to_capture`：1.0、2.0 Retina、非零 crop origin、负/多显示器 origin 共 4 例。
- 匹配器：完全重叠+同文本 => 高置信度；bbox 不相交 => 无匹配；文本不符 => 置信度下降；同 leaf 取最优元素。
- fallback 契约：`NoSemanticProvider` 恒返回 `Err(Unavailable)`（R13 可测）。
- 场景路由：Table / UiList / UiControl / DocumentParagraph 各分支、低置信度 => Unknown 门禁、无信号 => Unknown、Table 与散落 Button 不互相误路由；Unknown => `RenderFlow::ExactLine` 意图。
- 命令输出：`cargo test replace_translation::semantic` 9 passed / 0 failed；`cargo test replace_translation::routing` 7 passed / 0 failed；`cargo build`（含 `#[cfg(macos)]` AX provider）clean（唯一 warning 为无关既有 `plugins/install.rs::bundle_summary` dead_code）；`cargo fmt -- --check` exit 0。

### 编译通过但本机未运行时验证（真机缺口）

- `MacosAxProvider`：在 macOS 上编译通过，但对真实应用的 AX 遍历（`AXUIElementCreateSystemWide` -> 焦点应用/窗口 -> children，读 AXRole/AXValue/AXPosition/AXSize，超时与权限映射）需要真机 + 已授予 Accessibility 权限才能验证。无法在此单测覆盖；已在源码注释与本节标注为真机门禁项。

### 完全未做（本会话明确排除）

- Windows UI Automation provider（计划 Step 3）：本会话未写任何 `#[cfg(target_os = "windows")]` 代码；`SemanticSource::WindowsUiAutomation` 仅作为枚举占位。
- 轻量文档布局 ML 模型评估/转换（计划 Step 6）：未做。
- 把语义硬边界接入 `lens_commands.rs` 的 translation-group / render-slot 生成（计划 Step 7）：未接线；`classify_scene`/`match_leaves_to_elements`/provider 均未被实时管线调用，`render_flow_for_scene` 仅表达未来接线的意图。
- 双平台实机视觉门禁（计划 Step 8）：未做，仍归 visual-benchmark 最终关闭。

结论：本切片交付物为**纯新增、可编译、确定性逻辑全绿**的语义几何 + 场景路由基础模块；AC-S1/S2/S3/S5 的真机侧与 Windows 侧、以及实时接线均**未在本会话闭合**。
