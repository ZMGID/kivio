# Chat GUI 无头测试通道（probe）

**仅 debug 构建**。让自动化 / 手动通过文件驱动**运行中的 Kivio GUI 客户端**跑真实的 chat 生成，捕获模型实际调用的工具 + 回答——用于验证工具改动（如改名后模型是否还能调对）、回归测试，免去手点 GUI。

release 构建里整套 probe 被 `#[cfg(debug_assertions)]` 编译掉，不存在。

## 用法

1. `npm run dev` 起 app（debug）。启动日志会打印：
   `[chat-probe] watching <app_data>/chat_probe/request.json (debug-only test channel)`
2. 确认没有尚未处理的 `request.json`，再写入 `<app_data>/chat_probe/request.json`。默认 Dev 配置在 Windows 使用 `%APPDATA%\com.zmair.kivio.dev\chat_probe\request.json`；自定义配置以启动日志为准。测试不要指向正式版数据目录。
   ```json
   {
     "id": "my-test-1",
     "prompt": "用 glob 查找当前目录中的 Rust 源文件，报告找到的文件数",
     "cwd": "E:/path/to/test-fixture"
   }
   ```
   可选字段 `provider`、`model` 省略时使用默认聊天模型；`skillId` 选择技能，`cwd` 指定文件工具根目录。续聊时传回上一次结果的 `conversationId`，并保持相同的测试目录。
3. watcher（~700ms 轮询，按 mtime 去抖）拾取后，把 `request.json` 重命名为 `request.consumed`，经 `commands::run_chat_probe` 和 `complete_assistant_reply_inner` 调用应用 `run_session`，再进入共享 SDK 循环。完成后写：
   - `<app_data>/chat_probe/result.json`（最新一次）
   - `<app_data>/chat_probe/result-<id>.json`（带 `id` 时）
   ```json
   {
     "id": "my-test-1",
     "answer": "……",
     "toolCalls": [{ "name": "glob", "arguments": "{...}", "status": "success" }],
     "streamOutcome": "completed",
     "error": null,
     "finishedAt": 1730000000
   }
   ```

## 观察 / 调试

每次 probe 的会话都**保留在会话列表**里（挂在自动创建的 **「Chat Probe」项目**下，标题 `🔬 <prompt 前 60 字>`），可在 GUI 里点开看完整消息 / 工具调用 / 流式轨迹。**不隔离、不删除**。

## 行为要点

- **无头自动放行**：probe 生成用 `ProbeAgentHost` + `approval_policy=auto`——工具审批 / 会话 consent 自动允许，`ask_user` 自动返回取消态（不阻塞）。仅作用于 probe 这次生成，**不改全局 settings、不影响用户真实会话的审批门**。
- **超时兜底**：单次生成 360s 超时也会写出带错误的 `result.json`，watcher 不会永久卡住。
- **cwd → 文件工具根**：非项目会话是 global workspace 无根，相对路径不解析（与真实 GUI 一致）。probe 把会话绑到「Chat Probe」项目、根设为 `cwd`，使 `read`/`glob`/`grep` 的相对路径可解析。
- **串行**：watcher 一次处理一个请求。
- **跨 worktree 共用 Dev 身份**：不同 checkout 的 Dev 应用仍共用单实例、端口和 probe 目录。开始前确认进程路径并协调独占验证；结束后只关闭本次启动的进程。仅凭请求 ID 不能证明消费它的是预期构建。

## 已知限制 / 备忘

- 模型端点可能拒绝特定请求字段；probe 会保留实际错误。应用已有 Gemini 原生适配器，使用 Gemini 时应核对供应商的协议配置，不能由 probe 的聚合错误判断是哪一层故障。
- probe 只记最终聚合结果（answer + tool_records），不记流式逐帧。
- 这是实际 Dev 应用中的生成和持久化验证，不代替审批 UI、窗口交互或 OMP Hub 状态的手动检查。
