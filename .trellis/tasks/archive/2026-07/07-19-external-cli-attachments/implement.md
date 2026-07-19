# Implement — 本地 CLI 附件/图片打通

执行顺序按「先打通高收益协议、每步可编译可测」。每步做完跑 `cargo check --manifest-path src-tauri/Cargo.toml`。

## 步骤

### S1. 共享工具层 `external_agents/attachments.rs`
- [ ] 新建文件：`ImageBlock` struct、`load_image_blocks(paths)`、`image_paths_note`、`file_attachments_note`、`mime_for_ext`。
- [ ] 在 `external_agents/mod.rs` 挂 `mod attachments;`。
- [ ] 单测：mime 映射、空输入返回空、缺失文件返回 Err、note 拼装格式。
- 验证：`cargo test attachments`

### S1b. `RuntimeAgentDef` 图片能力标记
- [ ] `types.rs::RuntimeAgentDef` 加 `supports_native_image: bool` + `image_mime_whitelist: &'static [&'static str]`。
- [ ] 各 def 填值：claude=true+白名单{jpeg,png,gif,webp}；grok/acp=true+空；codex=true+空；pi/kimi=false+空。
- 验证：`cargo check`（所有 def 字面量补齐字段）

### S2. 线程化图片路径入口（reply.rs → run.rs）
- [ ] `reply.rs` 外部分支：取最后一条 user 消息的 image 附件 → `stored_image_paths_for_attachments`（已存在）→ 传入。当前分支已能拿到 `conversation`；`stored_image_paths_for_attachments(app,&conv.id,&msg.attachments)`。
- [ ] `run_external_cli_reply` 新增参数 `image_paths: &[PathBuf]`；`run_external_cli_slash_command` 传 `&[]`。
- [ ] `run.rs` 内 `is_slash` 时忽略 image_paths（R5）。
- [ ] 早期 `load_image_blocks` 失败 → 返回可见错误（R6）。
- 验证：`cargo check`

### S3. Claude（StreamJson，非持久，最高收益之一）
- [ ] `spawn.rs::stream_json_user_content` 改签名接收 `images: &[ImageBlock]`，非 slash 时把 image 块追加进 content 数组。
- [ ] **mime 白名单过滤**：不在 `image_mime_whitelist` 的图片不注入，改由 §6 降级为路径提示（不静默丢）。
- [ ] `write_prompt_stdin` 透传 images；`run.rs` 调用处传本轮 images。
- [ ] 更新既有单测（无图分支断言不变）+ 新增带图断言 + 白名单外降级断言。
- 验证：`cargo test`；手动：Claude 会话发 png 问「这是什么」。 (AC2)

### S4. ACP（grok/generic，持久，截图问题主角）
- [ ] `SessionCommand::RunTurn` 加 `images: Vec<ImageBlock>` 字段。
- [ ] `run.rs::run_persistent_turn` 签名 + `RunTurn` 构造带 images。
- [ ] `run_acp_session`（非持久早期路径）与 `AcpSession::run_turn`（持久）在 `session/prompt` 数组注入 `{type:"image",data,mimeType}`。
- [ ] 单测：prompt 数组构造带图。
- 验证：`cargo test`；手动：grok 会话发图。 (AC1)

### S5. Codex（CodexAppServer，持久，需临时文件）
- [ ] `attachments.rs` 增 `materialize_images_to_tempdir(images) -> Vec<PathBuf>`（mkdtemp 0700 + sha256 命名 0600，前缀 `kivio-ext-img-`）。
- [ ] `CodexAppServerSession::run_turn` 接收 images，`turn/start` 的 `input` 追加 `{type:"localImage",path}`。
- [ ] `run.rs` 持久路径把 images 交给 codex actor。
- [ ] `cleanup_orphan_temp_files` 纳入 `kivio-ext-img-*` 前缀 GC（screenshot.rs）。
- 验证：`cargo test`；手动：Codex 会话发图。 (AC3)

### S6. 图片降级 + 文件附件（所有协议）
- [ ] `run.rs`：对不支持图片块的协议（PiRpc、JsonEventStream）把 `image_paths_note` 追加进 `composed.full_prompt`。
- [ ] `run.rs`：对**所有**协议，把非图片文件附件用 `file_attachments_note`（文件名/路径/MIME/大小）追加进 prompt（对齐 Paseo `uploaded_file`）。
- [ ] `extra_allowed_dirs_for_agent` 或 `runtime_ctx` 组装处：加入本会话附件目录（codex 除外）。
- 验证：`cargo check`；手动：pi/kimi 发图 prompt 含路径；任意会话发 .pdf/.txt，prompt 含文件元信息且 CLI 可读。 (AC4, AC4b)

### S7. 收尾
- [ ] 全量 `cargo test --manifest-path src-tauri/Cargo.toml`（对齐 CLAUDE.md 里已知 baseline 失败，勿计回归）。
- [ ] `npm run lint && npm run typecheck`（若前端无改动可跳，但确认没顺手动到）。
- [ ] 逐条核对 AC1–AC7。
- [ ] spec 更新（3.3）+ commit（3.4，Conventional Commits: `feat(external-agents): ...`）。

## 验证命令汇总
- `cargo check --manifest-path src-tauri/Cargo.toml`
- `cargo test --manifest-path src-tauri/Cargo.toml external_agents`
- 手动冒烟：grok / Claude / Codex 各发一张图。

## 回滚点
- 每个 S 步独立可编译；出问题回退到上一步 commit。图片注入全部 gated on 非空 `image_paths`，最坏情况回退到「纯文本，附件仍被丢」的现状，不会更糟。
