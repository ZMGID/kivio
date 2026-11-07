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
   可选字段 `provider`、`model` 省略时使用默认聊天模型；`skillId` 选择技能，`cwd` 指定文件工具根目录。`timeoutSeconds` 指定长任务超时（默认 360 秒，限制在 1–3600 秒）。续聊时传回上一次结果的 `conversationId`，并保持相同的测试目录。
3. watcher（~700ms 轮询，按 mtime 去抖）拾取后，把 `request.json` 重命名为 `request.consumed`。默认经 `commands::run_chat_probe` 和 `complete_assistant_reply_inner` 直接调用生成核心；`sendThroughIpc: true` 则由已打开的聊天或主 WebView 调用 `chat_send_message`，覆盖真实 IPC 包装、发送预留和用户消息落盘。完成后写：
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

## 验证真实 IPC 发送

启动 Dev 应用并打开聊天窗口，再运行：

```sh
node scripts/probe-chat-send.mjs --app-pid <Dev应用进程ID>
```

脚本在同一条内置运行时对话中验证两次完成、一次取消及取消后的再次发送，并核对每轮持久化消息。`--app-pid` 可选，指定后应用退出会立即失败；未指定时按有界等待判定失败。脚本不会跳过模型、应用或超时错误。

使用 `--probe-dir PATH` 或 `KIVIO_PROBE_DIR` 指向启动日志中的目录。`--provider ID`、`--model NAME` 可覆盖新对话的模型，`--timeout-seconds` 控制每轮上限。脚本拒绝覆盖尚未消费的请求；会话、结果文件和临时工作目录保留供检查。`node scripts/probe-chat-send.mjs --self-test` 只验证请求发布与消息断言，不连接应用。

手工请求可组合 `sendThroughIpc: true`、`skillId`、`cancelAfterMs`、`computeContextStats` 和已有的模式、联网搜索、外部运行时覆盖。IPC 路径由生产命令创建用户消息，probe 不重复插入；`conversationId` 仍用于续聊。

Windows 工具栈回归使用独立 Test 构建和本机模拟模型、MCP 服务，不使用用户的服务商或密钥。先按 README 完成构建前置步骤，在一个终端运行 `npm run dev:ui`，另一个 PowerShell 终端执行：

```powershell
$env:TAURI_CONFIG = Get-Content src-tauri/tauri.test.conf.json -Raw
cargo build --manifest-path src-tauri/Cargo.toml --bin kivio --no-default-features
Remove-Item Env:TAURI_CONFIG
node scripts/probe-chat-tools.mjs --exe src-tauri/target/debug/kivio.exe --app-data "$env:APPDATA\com.zmair.kivio.test"
```

脚本先检查可执行文件的 Test 产品标识及编译身份，再启动测试。它仅接受 `com.zmair.kivio.test` 数据目录，首次运行会创建目录和所有权标记；已有目录必须带 `.kivio-stack-fixture.json` 标记，内容为 `{"kind":"kivio-stack-fixture","identifier":"com.zmair.kivio.test","version":1}`。正式版与 Dev 设置不受影响。

脚本启动并关闭本次 Test 进程，经真实 IPC 验证 native `web_search` → Exa MCP、直接 MCP、两个安全工具并行、等待工具时取消后恢复、工具失败后重试。每项核对 MCP 服务端调用和持久化工具记录，纯文本回答不能通过；取消后的迟到结果也不能改写已取消消息。日志与结果保存在 Test 数据目录的 `stack-fixture` 下。`--self-test` 只检查模拟服务协议和请求发布，不启动应用。

Windows Dev 构建使用仓库的 `[profile.dev] opt-level = 1`，保留调试信息与断言。仅优化依赖不能控制应用内实例化的泛型代码，见 [Cargo profiles](https://doc.rust-lang.org/cargo/reference/profiles.html#overrides-and-generics)。不要通过环境变量将应用优化级别降回 0 后沿用本验收结果。基础优化会增加构建时间，并影响调试器逐行执行的对应关系。

工具回归还要求工作线程栈保持默认 2 MiB，且 native MCP 连接入口的每个采样至少剩余 1 MiB。该断言监测已复现路径的栈余量，不代表全程峰值或所有工具的栈上界。模拟服务验证真实客户端的协议与执行路径；真实服务商、其他工具和 release 安装包需要单独验证。

## 观察 / 调试

每次 probe 的会话都**保留在会话列表**里（挂在自动创建的 **「Chat Probe」项目**下，标题 `🔬 <prompt 前 60 字>`），可在 GUI 里点开看完整消息 / 工具调用 / 流式轨迹。**不隔离、不删除**。

## 行为要点

- **无头自动放行**：默认直接生成用 `ProbeAgentHost` + `approval_policy=auto`，工具审批与会话 consent 自动允许，`ask_user` 自动返回取消态。`sendThroughIpc: true` 保留生产审批行为，不能把它当作自动放行通道。两种路径都不修改全局设置。
- **超时兜底**：单次生成默认 360s 超时，可用 `timeoutSeconds` 延长至一小时；超时也会写出带错误的 `result.json`，watcher 不会永久卡住。
- **IPC 清理**：等待 IPC 结果时超时会取消该对话的生产生成并解除结果监听；`cancelAfterMs` 的计时任务随本次 probe 结束而中止，不会取消下一轮。
- **cwd → 文件工具根**：非项目会话是 global workspace 无根，相对路径不解析（与真实 GUI 一致）。probe 把会话绑到「Chat Probe」项目、根设为 `cwd`，使 `read`/`glob`/`grep` 的相对路径可解析。
- **串行**：watcher 一次处理一个请求。
- **跨 worktree 共用 Dev 身份**：不同 checkout 的 Dev 应用仍共用单实例、端口和 probe 目录。开始前确认进程路径并协调独占验证；结束后只关闭本次启动的进程。仅凭请求 ID 不能证明消费它的是预期构建。

## 已知限制 / 备忘

- 模型端点可能拒绝特定请求字段；probe 会保留实际错误。应用已有 Gemini 原生适配器，使用 Gemini 时应核对供应商的协议配置，不能由 probe 的聚合错误判断是哪一层故障。
- probe 只记最终聚合结果（answer + tool_records），不记流式逐帧。
- 这是实际 Dev 应用中的生成和持久化验证，不代替审批 UI、窗口交互或 OMP Hub 状态的手动检查。
