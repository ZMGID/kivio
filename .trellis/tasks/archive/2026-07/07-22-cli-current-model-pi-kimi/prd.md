# PRD — pi/kimi 当前模型同步显示

用户反馈（2026-07-22 真机验收后）：pi/kimi 胶囊显示 Auto，但两个 CLI 本地配置里明明有"当前默认模型"——能探测到就该显示真实模型名，与 codex（读 config.toml）同等待遇。

## 本机已核实的配置来源

- **pi**：`~/.pi/agent/settings.json` → `defaultProvider` + `defaultModel`（本机实测 `edgefn` / `DeepSeek-V4-Flash`）。pi 的模型 id 形态是 `provider/model`（如 `edgefn/DeepSeek-V4-Flash`）。
- **kimi**：`~/.kimi-code/config.toml` 顶层 `default_model = "kimi-code/kimi-for-coding"`；另有 `[thinking] enabled/effort`（本机 high）——kimi 走 ACP 但 session/new 不上报 currentModelId，故从本地配置读。

## Requirements

- `detect_agent_models` 的 current_model/current_reasoning 提取扩展两个分支：
  - pi：读 settings.json 的 defaultProvider/defaultModel，拼成 `provider/model` 作为 currentModel（与 pi --list-models 的 id 形态一致，能回填选择）。
  - kimi：ACP 探测无 currentModelId 时降级读 config.toml 顶层 `default_model` 作 currentModel；`[thinking].effort`（enabled=true 时）作 currentReasoning。注意 kimi 的 toml 解析需进 section（[thinking]），现有 codex 顶层扫描器需小扩展或单独小函数。
- 复用现有机制：currentModel 存在且在探测列表中 → 前端自动回填选择；不在列表 → 仅展示（同 claude 取舍）。
- 配置文件缺失/键缺失 → None → 现状 Auto（不回归）。

## Acceptance Criteria

- [ ] 单测：pi settings.json 解析（正常/缺键/缺文件）；kimi config.toml 的 default_model + [thinking] 解析（含 section 边界）。
- [ ] 实测：pi 新会话胶囊显示 DeepSeek-V4-Flash（或当时配置值）；kimi 显示 kimi-for-coding + High。
- [ ] `cargo test --lib` + 前端命令全绿（前端预计零改动——机制已就位）。
