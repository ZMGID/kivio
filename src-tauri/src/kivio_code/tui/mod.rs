//! Kivio Code TUI —— PI 风格的「行级差分渲染」终端 UI 库。
//!
//! 与 ratatui 的 cell-grid/back-buffer 模型不同，本库完全以「每帧一组 ANSI 字符串行」
//! 为模型：每个 [`Component`] 的 `render(width)` 返回 `Vec<String>`（每项 = 一终端行），
//! 整棵组件树拼成行数组，渲染器在帧间 diff *行数组*，只发出最小的相对光标移动 + 改动行重写，
//! 全程包在 synchronized-output（`\x1b[?2026h/l`）里以避免撕裂。渲染进 NORMAL buffer
//! （不进 alt-screen），让内容自然滚入 scrollback —— 对标 PI 的 `tui.ts`。
//!
//! 依赖顺序（每层依赖上一层）：
//! 1. [`text_width`] —— ANSI-aware 列宽 / 截断 / 切片 / wrap。**一切的基石。**
//! 2. [`terminal`] —— 终端抽象（trait + 可测试的 fake）。
//! 3. [`stdin_buffer`] —— 把原始字节切成「完整转义序列」。
//! 4. [`keys`] / [`keybindings`] —— 字节序列 → `Key` 解码 + 动作绑定表。
//! 5. [`render`] —— 差分行渲染器（`Component`/`Container`/`Tui`/`do_render`）。
//! 6. [`components`] —— `Text` / `BoxView` / `Spacer` / `TruncatedText`。

pub mod components;
pub mod keybindings;
pub mod keys;
pub mod render;
pub mod stdin_buffer;
pub mod terminal;
pub mod text_width;
