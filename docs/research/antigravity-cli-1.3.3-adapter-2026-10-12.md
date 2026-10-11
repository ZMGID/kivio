# Antigravity CLI 1.3.3 适配与验证记录

日期：2026-10-12。未创建 Trellis 任务，未启动子代理，未更新本机已安装的 CLI。

## 核查依据

- [官方 1.3.3 release](https://github.com/google-antigravity/antigravity-cli/releases/tag/1.3.3)，2026-10-10 发布。
- [官方 headless 协议](https://antigravity.google/docs/cli/headless/)：`step_update.usage` 是单步消耗，`result.usage` 是会话累计消耗；`checkpoint` 与 `agent_response` 是不同步骤。
- [CLI 更新记录](https://www.antigravity.google/docs/changelog?tab=cli)：1.2.1 加入内部重试；1.2.8/1.2.9 修复压缩与流响应问题。该页面本次仍标记 1.3.1 为最新，应以官方 release 为准。

## 修复后的行为

- 计费统计仍汇总一轮所有步骤，包括 checkpoint；不覆盖或篡改累计消耗。
- 最近完成的 `agent_response` 单步 `input + cache_read + output` 单独保存为 `ModelUsage.context_tokens`，供上下文面板读取。它是最近模型请求的占用口径，不是 CLI 的实时活动上下文快照。
- checkpoint 的模型调用消耗不作为活动上下文。不能把普通 checkpoint 推断成“已压缩”，也不推算压缩次数或压缩后大小。
- 后续请求变小或恢复原生会话后，面板读取新的单步占用；不继续叠加历史请求。
- 旧 Antigravity 消息没有该字段时显示未知，直到新的请求提供测量；避免恢复旧的 294%/529% 虚高统计。旧存档无需改写。
- 主窗口与弹出窗口都显示禁用的“CLI 自动管理”动作；后端也明确当前流式接口不支持手动压缩。`/context` 是交互式用量面板。
- 连接被远端关闭、认证过期、额度耗尽分别归类；保留 CLI 内部重试和工具结果去重，不在 Kivio 中自动重放整个任务。
- `xhigh`/`max` 强度选项来自 1.3.3 的真实帮助输出；对应模型变体不再被遗留的独立强度覆盖。

新增字段的其他 provider/CLI 构造位置只补 `None`，保持原有统计行为。字段使用 serde 默认值并在空值时省略，不改变旧存档读取，也不修改生成的运行事件协议。

## 真实 1.3.3 验证及阻塞

从官方 release 下载 macOS ARM64 二进制到隔离临时目录，使用 `AGY_CLI_DISABLE_AUTO_UPDATE=true` 启动。

- `--version` 实际输出 `1.3.3`。
- `--help` 实际包含 `--effort ... (low|medium|high|xhigh|max)`，`--print-timeout` 默认为 `0s`。
- 在隔离工作目录输入两轮 NDJSON，尝试验证记忆与只读文件工具；进程退出码为 1，没有 init，也没有模型或工具执行。
- 真实 stdout：

```json
{"event":"result","result":{"conversation_id":"","status":"ERROR","response":"","error":"authentication failed or timed out","duration_seconds":0,"num_turns":0,"usage":{"input_tokens":0,"output_tokens":0,"thinking_tokens":0,"cache_read_tokens":0,"total_tokens":0}}}
```

stderr 包含 `authentication required`。用户选择先完成修复并记录阻塞。因此正常会话、真实自动压缩、断流后恢复以及完整原生窗口流程尚未完成实机验收，不能以合成回归用例代替。

已有的多轮/工具/取消/原生恢复实测用例增加了请求占用断言，并可通过测试专用 `KIVIO_AGY_TEST_BIN` 指定隔离二进制。登录后可运行：

```sh
KIVIO_AGY_TEST_BIN=/absolute/path/to/antigravity-1.3.3 cargo test --manifest-path src-tauri/Cargo.toml live_antigravity_multiturn_tools_cancel_and_resume --lib -- --ignored --nocapture
```

该用例验证多轮、工具、取消和原生恢复，不强制触发自动压缩。自动压缩需要额外采集已登录长会话的真实流，确认有无可用的压缩元数据，再判断是否能显示原生压缩边界。

## 回归验证

统计分离、按钮能力和新增强度的回归用例已在修复前失败、修复后通过；上下文存档往返覆盖新字段与旧记录的未知回退。另运行 CLI 相关 Rust 测试、上下文/错误分类测试、前端相关测试、定向 ESLint、架构检查、协议检查和前端构建。

- 外部 CLI Rust 测试：804 通过、47 忽略（包括需要登录的实测）。
- 前端相关测试：6 个文件、72 项通过。
- 定向 ESLint、架构检查、协议检查、UI 构建与 `git diff --check` 通过。

默认 `tsc --noEmit` 存在仓库原有的 ES library 配置错误（`.at`/`replaceAll`）；使用 `--lib es2022,dom,dom.iterable` 检查实际类型错误。不在本次修改全局 TypeScript 配置。
