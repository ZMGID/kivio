# 把快速翻译从 lens 窗口拆成独立窗口

## Goal

让"快速翻译"（截图翻译 / 选词翻译）使用**独立的 webview 窗口**，与 lens 截图问答（chat）解耦。当前三者中 lens 问答 + 截图翻译 + 选词翻译共用同一个 `lens` 窗口，按 `#lens?mode=` 渲染；它们的状态/生命周期耦合，本轮一连串 bug 都集中在这个共享窗口上。拆开后各自独立，互不干扰、更稳。

## What I already know（代码事实）

- 共用 `lens` 窗口的入口（都走 `lens_request_internal` → `windows::ensure_lens_window`，lens_commands.rs）：
  - `lens_request`（mode=chat，截图问答）
  - `lens_request_translate`（mode=translate，截图翻译）
  - `lens_request_translate_text`（mode=translateText，选词翻译）
- `toggle_main_window`（输入翻译）→ `main` 窗口，**与本任务无关**。
- 前端 `src/Lens.tsx` 单文件按 `readModeFromHash()` 的 mode 渲染：select 全屏选区 / ready 浮条 / answering / translate 结果卡。截图翻译和 lens 问答**共用 select 选区 overlay + 截图 + 历史 + 结果区**逻辑。
- 这些命令当前**硬编码** `get_webview_window("lens")`，跨窗口需改成按调用窗口路由：`lens_capture_region` / `lens_translate` / `lens_set_floating` / `lens_animate_floating` / `lens_close` / `lens_focus_webview` / `lens_register_annotated_image`（`lens_ask` 只 chat、`lens_translate` 只 translate）。
- 窗口都已转**非激活 NSPanel**（`ensure_overlay_panel`/`configure_overlay_panel`：MoveToActiveSpace|FullScreenAuxiliary|Transient|IgnoresCycle、status level、hidesOnDeactivate=NO、hasShadow=NO、_setPreventsActivation、_isNonactivatingPanel、WKWebView first-responder）。新窗口要复用同一套。
- `AppState.lens_busy: AtomicBool` 单标志、`pending_selection`、`lens_freeze_frame_image_id`（Windows）、`current_explain_image_id`、`explain_stream_generation` 等都是 lens 单窗口假设。
- capabilities/default.json 的 windows 白名单当前 `["main","chat","settings","lens"]`，加窗口必须登记新 label，否则插件调用静默失败。
- 热键独立：lens 热键 / 截图翻译热键 / 选词翻译热键在 settings 里各自配置（shortcuts.rs register_hotkeys）。

## 候选实现骨架（待 brainstorm 确认）

- 新窗口 label（如 `translate`），加入 capabilities 白名单；`ensure_translate_window`（镜像 `ensure_lens_window` + `ensure_overlay_panel`）。
- `lens_request_internal` 参数化目标窗口 label（lens vs translate），或拆出并行函数；截图翻译/选词翻译路由到 translate 窗口。
- 共享命令改为操作**调用窗口**（Tauri 注入 `WebviewWindow`），不再硬编码 "lens"。
- 状态隔离：lens_busy 是否拆成 lens/translate 两个？stream generation / current image / pending_selection 是否要按窗口隔离？（关键设计点）
- 前端 Lens.tsx 复用：两个窗口跑同一 bundle，按 mode 渲染；`lens:reset`/事件按窗口隔离。

## Open Questions（仅 Blocking / Preference）

- ~~范围：截图翻译 + 选词翻译都拆？~~ → **都拆**（lens 窗口只留问答）。
- ~~lens 与 translate 是否允许同时打开？~~ → **同一时刻只活跃一个**（触发一个时收起另一个），复用底层状态，不做全量状态隔离。

## Decision (ADR-lite)

- **Context**: 快速翻译（截图 + 选词）与 lens 问答共用 `lens` 窗口、共享 Lens.tsx 与 AppState，耦合重。
- **Decision**: 新增独立 `translate` 窗口承载快速翻译；与 lens 问答**互斥**（同一时刻只一个可见）。复用 Lens.tsx 同一 bundle（按 mode 渲染）+ 复用 AppState 单份状态（互斥保证不冲突）。共享命令改为操作"当前活动的浮窗"（lens 或 translate）。
- **Consequences**: 改动集中在窗口创建 + 路由 + 命令的窗口寻址；不需拆 busy/流/图状态（互斥下安全），风险与工作量可控。UI/生命周期解耦达到目的。

## Implementation Plan（小步）

1. **新窗口骨架**：capabilities/default.json 白名单加 `translate`；`windows.rs` 加 `ensure_translate_window`（镜像 `ensure_lens_window`：borderless/transparent/shadow(false)/visible(false) + `ensure_overlay_panel`），`get_*` 辅助。
2. **命令窗口寻址**：把硬编码 `get_webview_window("lens")` 的共享命令改为操作"当前活动浮窗"——加 `active_overlay_window(app)`（返回 lens/translate 中可见的那个），用于 `lens_capture_region`/`lens_translate`/`lens_set_floating`/`lens_animate_floating`/`lens_close`/`lens_focus_webview`/`lens_register_annotated_image`。`lens_ask`（仅 chat）→ lens；`lens_translate`（仅 translate）→ translate。
3. **入口路由 + 互斥**：`lens_request_internal` 按 mode 选目标窗口（chat→lens，translate/translateText→translate）；显示目标前先关掉另一个浮窗（互斥）。`lens_is_active`/`focus_lens_window`/Esc 全局快捷键/`Focused(true)` 自愈/tray 都按"活动浮窗"泛化。
4. **前端**：translate 窗口跑同一 Lens.tsx（mode=translate/translateText），按 mode 只渲染翻译 UI；`lens_focus_webview` 重试聚焦同样接上（命令已按调用窗口路由）。Windows 冻结帧/prewarm 在新窗口上对齐。
5. **验证**：cargo check/test、typecheck/lint；macOS 手动冒烟（截图翻译/选词翻译在新窗口全流程 + 全屏/键盘/跨 Space/阴影；lens 问答不受影响；互斥切换正常）。

## Requirements (evolving)

- 截图翻译（至少）使用独立于 lens 的 webview 窗口；lens 问答仍用 `lens` 窗口。
- 不回归本轮已修：全屏浮现、键盘焦点、跨 Space、无原生阴影、不崩溃——新窗口同样满足。
- 输入翻译（main）不受影响。

## Acceptance Criteria (evolving)

- [ ] 截图翻译在独立窗口里完成选区→截图→OCR/翻译→结果卡，全程正常。
- [ ] lens 问答与快速翻译互不干扰（一个开着不影响另一个）。
- [ ] 新窗口在 macOS 上：全屏浮现 / 键盘 / 跨 Space / 无怪阴影 / 不崩 都正常。
- [ ] `cargo check`/`cargo test`/`typecheck`/`lint` 通过。

## Out of Scope

- 输入翻译（main 窗口）。
- 翻译算法 / OCR 引擎逻辑（不动）。

## Technical Notes

- 关键耦合点：lens_commands.rs 的共享命令硬编码 "lens"；AppState 的 lens 单窗口状态；capabilities 白名单；前端 Lens.tsx 单 bundle 多 mode。
- 复用本轮的 spec：`.trellis/spec/backend/window-lifecycle.md`（NSPanel / 键盘 / 跨 Space / 阴影契约）。
