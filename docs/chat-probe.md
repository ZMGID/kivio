# Chat GUI 无头测试通道（probe）

**仅 debug 构建**。让自动化 / 手动通过文件驱动**运行中的 Kivio GUI 客户端**跑真实的 chat 生成，捕获模型实际调用的工具 + 回答——用于验证工具改动（如改名后模型是否还能调对）、回归测试，免去手点 GUI。

release 构建里整套 probe 被 `#[cfg(debug_assertions)]` 编译掉，不存在。

## 用法

1. `npm run dev` 起 app（debug）。启动日志会打印：
   `[chat-probe] watching <app_data>/chat_probe/request.json (debug-only test channel)`
2. 写请求文件 `<app_data>/chat_probe/request.json`（Windows：`%APPDATA%\com.zmair.kivio\chat_probe\request.json`）：
   ```json
   {
     "id": "my-test-1",
     "prompt": "用 glob 查找 src/native_tools/*.rs，报告有几个文件",
     "provider": "provider-xxxx",   // 可选；省略用默认 chat provider
     "model": "grok-composer-2.5-fast", // 可选；省略用默认 chat model
     "skillId": "pdf",              // 可选
     "cwd": "E:/path/to/repo"       // 可选；文件工具相对路径的根。省略 = 进程 cwd（dev 通常是 src-tauri）
   }
   ```
3. watcher（~700ms 轮询，按 mtime 去抖）拾取后，把 `request.json` 重命名为 `request.consumed`，走**与聊天窗口完全相同的生成路径**（`chat_send_message`/`complete_assistant_reply_inner` → `run_agent_loop` + 全量工具集），完成后写：
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
- **超时兜底**：单次生成 120s 超时也会写出 `result.json`（`error:"timeout"`），watcher 不会永久卡住。
- **cwd → 文件工具根**：非项目会话是 global workspace 无根，相对路径不解析（与真实 GUI 一致）。probe 把会话绑到「Chat Probe」项目、根设为 `cwd`，使 `read`/`glob`/`grep` 的相对路径可解析。
- **串行**：watcher 一次处理一个请求。

## 已知限制 / 备忘

- 某些 provider 端点不接受 Kivio 发的 `prompt_cache_key`/`promptCacheKey` 会话缓存键字段（实测 Google Gemini OpenAI-compat 端点返回 `400 Unknown name "promptCacheKey"`）。probe 会把该错误如实写进 `result.json.error`。**这不是 Kivio 的 bug**——opencode 用同样的 url+key 也复现完全相同报错，是 OpenAI 风格客户端普遍撞 Gemini OpenAI-compat shim 严格校验的结果。正确方向是**以后为 Gemini 做原生接口协议适配**（peer adapter），待单独任务。
- probe 只记最终聚合结果（answer + tool_records），不记流式逐帧。
