# Design — 本地 CLI 附件/图片打通

## 1. 边界与契约

新增一个统一的「图片附件 → 各协议原生块」转换层，镜像 Paseo 的 `buildAgentPrompt` → per-provider adapter。kivio 侧不搞协议无关中间层的重抽象（YAGNI），直接在各 session 函数注入点转换，共享两个小工具函数。

### 数据流（改后）

```
send.rs (已算 last_user_image_paths)
  → reply.rs 外部分支：把 image_paths 一起传下去   [改]
    → run_external_cli_reply(image_paths)           [新增参数]
      → 非持久: write_prompt_stdin / run_pi_rpc_session(image_paths)
      → 持久:   run_persistent_turn → SessionCommand::RunTurn{ images }   [新增字段]
                 → AcpSession::run_turn / CodexAppServerSession::run_turn(images)
```

## 2. 共享工具（新文件 `external_agents/attachments.rs`）

```rust
pub struct ImageBlock { pub data_base64: String, pub mime: String, pub path: PathBuf }

/// 读磁盘图片 → base64 + mime（按扩展名）。失败返回 Err。
pub fn load_image_blocks(paths: &[PathBuf]) -> Result<Vec<ImageBlock>, String>;

/// 图片降级（PiRpc/kimi）：把图片绝对路径拼成一段可加进 prompt 的文本。
pub fn image_paths_note(paths: &[PathBuf]) -> String;  // "" if empty

/// 非图片文件（所有协议，Paseo 对等）：渲染「文件名/路径/MIME/大小」文本块。
pub fn file_attachments_note(files: &[FileAttachment]) -> String;  // "" if empty
```

分类：附件按 `attachment_type == "image"` 分成 images 与 files 两组。images 走原生块（支持的协议）或图片降级（pi/kimi）；files **一律**走 `file_attachments_note`。

文件文本块格式（对齐 Paseo `uploaded_file`）：
```
Attached file: <name>
Path: <abs path>
MIME: <mime>
Size: <bytes> bytes
```

- mime 映射：png/jpg/jpeg/gif/webp/bmp → `image/*`；未知按 `application/octet-stream`。
- base64 用已在依赖里的 `base64` crate（复用，不新增依赖）。

## 3. 各协议注入点（只加图片项，text 项保持原样）

### 图片能力标记（新增，镜像 Paseo 的 ad-hoc gating，但显式化）

Paseo 无统一 capability，靠各处硬判断。kivio 更干净的做法：`RuntimeAgentDef` 加一个字段：

```rust
supports_native_image: bool,   // claude/grok(acp)/codex = true; pi/kimi = false
image_mime_whitelist: &'static [&'static str],  // claude = ["image/jpeg","image/png","image/gif","image/webp"]; 空=不限
```

- `supports_native_image == false`（pi/kimi）：图片走**降级**（见 §5），不注入原生块。
- Claude：mime 不在白名单 → **不静默丢**（Paseo 是静默 drop，这是它的坑），改为把该图降级为路径提示（§5）。
- ACP/Codex：白名单为空，任意 mime 直接注入。

| 协议 | 文件:行 | 现状 | 改法 |
|---|---|---|---|
| Claude StreamJson | `spawn.rs:179 stream_json_user_content` | `[{type:text,text}]` | 追加 `{type:"image",source:{type:"base64",media_type:mime,data:b64}}`；slash（String 分支）不加 |
| ACP | `acp.rs:803/823` + `AcpSession::run_turn:988` | `"prompt":[{type:text,text}]` | 追加 `{type:"image",data:b64,mimeType:mime}` |
| Codex | `codex_app_server.rs:520 run_turn` | `"input":[{type:text,text}]` | 每图落临时文件→追加 `{type:"localImage",path}` |
| PiRpc | `pi_rpc.rs:341` | 文本 | 图片降级：`image_paths_note` 拼进 prompt |
| kimi JsonEventStream | 通用 stdin/stdout | 文本 | 图片降级：同上 |

**非图片文件（所有协议，含上面支持图片的三家）**：`file_attachments_note` 拼进 `composed.full_prompt` 末尾，+ allowed-dir。文件不 inline 内容，CLI 用 read 工具自读。

### Codex 临时文件

复用 Paseo 思路：`mkdtemp($TMPDIR/kivio-ext-img-XXXX)`（0700），每图写 `<sha256>.<ext>`（0600）。可直接引用会话已存的附件文件路径 —— 但 Codex sandbox 锁 cwd/allowed，稳妥起见落一份到临时目录再传 path（与 Paseo 一致）。临时目录随进程/退出清理（可复用现有 `cleanup_orphan_temp_files` 命名前缀约定）。

## 4. 线程化：持久会话

`SessionCommand::RunTurn` 现有字段 `{prompt, model, reasoning, events, done}`。**新增 `images: Vec<ImageBlock>`**（ACP/Codex actor 各自消费；Claude 非持久不经此路）。
- `run.rs::run_persistent_turn` 签名加 `images: &[ImageBlock]`，塞进 `RunTurn`。
- `AcpSession::run_turn` / `CodexAppServerSession::run_turn` 签名加 `images`，在构造 `session/prompt` / `turn/start` 时注入。
- 复用会话（reuse_prompt）与首轮（first_prompt）都带同一批 images（本轮 user 消息的图，天然只属于本轮）。

## 5. allowed-dirs（降级路径 + Claude/ACP 读文件兜底）

**图片降级**（`supports_native_image==false`，或 Claude mime 不在白名单）：把图片绝对路径拼进 prompt（`image_paths_note`，格式对齐 Paseo pi/omp 的 `[Image available at: {path}]`），并把附件目录加进 allowed-dir。比 Paseo 的静默丢弃更好。

**文件（uploaded_file 对等）**：`file_attachments_note` 拼进 prompt + allowed-dir。Paseo 传的是守护进程可见盘路径；kivio 附件已在 `conversation_attachments_dir(app,id)`，传该绝对路径即可，CLI 用自己的 read 工具读。

`extra_allowed_dirs_for_agent` 增加把「本会话附件目录」纳入（codex 仍返回空，因它走 localImage 不需要）。附件目录需传进 `run.rs` 组装 `runtime_ctx` 处。

## 6. 兼容性 / 防回归

- 所有改动对「无图片」输入是 no-op：`image_paths` 空 → content 数组不变 → 现有单测（`stream_json_user_content_uses_string_for_slash_commands` 等）逐字节通过。
- slash 命令：`is_slash` 时不取附件（R5）。
- 编码失败：`load_image_blocks` Err → 在 `run_external_cli_reply` 早期返回可见错误（R6），不静默继续。

## 7. Tradeoffs / 已知取舍

- **不做协议无关中间层**：kivio 只有 3 种支持图片的协议，直接在注入点转换比 Paseo 那套 `AgentPromptInput` 抽象更省（ponytail）。
- **文件附件（非图片）一律走降级**：结构化文件块各协议差异大、收益低，暂只做路径注入 + allowed-dir。
- **Codex 落临时文件**是唯一额外 I/O，接受（Paseo 也这么做）。
