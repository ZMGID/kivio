# 执行计划：翻译/截图/Lens 旧调用路径接入多协议模型适配器

前置阅读:本任务 `prd.md` + `design.md`(方案与决策全在 design.md,本文件只排执行顺序)。

## 步骤(按序,每步后 `cargo test --manifest-path src-tauri/Cargo.toml --lib` 应绿)

### 1. 基建:Cancelled 错误类型
- [ ] `chat/model/types.rs`:`ModelErrorKind` 加 `Cancelled` 变体;`ModelError` 加 `is_cancelled()` 辅助。
- [ ] usage 记录路径:四适配器 `record_usage_failure` 处,错误为 Cancelled 时 status 记 `"cancelled"` 而非 `"failure"`(实现方式见 design.md §3,尽量不改 4 处签名——可在共享辅助层判断)。

### 2. 分发函数搬迁(纯移动,无行为变化)
- [ ] `generate_with_chat_provider` / `stream_with_chat_provider` 从 `chat/agent/planning.rs` 移到 `chat/model/mod.rs`,改 `pub(crate)`;planning.rs 原调用点改 import。
- [ ] grep 确认无其他引用遗漏;编译绿。

### 3. 事件桥 sink + 单测
- [ ] `api.rs` 新增 `LensEventSink`(design.md §2:代际检查→Cancelled 错;TextDelta/ReasoningDelta/Finish → 现有事件 payload,形状逐字节不变)。
- [ ] 新增 combined-splitter 包装 sink:持有 `<<<ORIGINAL>>>` 拆分状态机(tail 缓冲、UTF-8 char boundary——从现 `stream_translate_combined` 内联逻辑提炼),TextDelta 切成 translated/original 两 kind。
- [ ] 单测:取消时首 emit 返回 Cancelled;分隔符跨 delta;分隔符前缀被 tail 扣住不误 emit;CJK 多字节边界。事件 emit 可用注入闭包/trait 抽象以便测试(sink 内不直接依赖 AppHandle 的部分尽量可测)。

### 4. 非流式入口迁移(先易)
- [ ] `call_openai_text`:内部改 GenerateRequest → `generate_with_chat_provider`;删 `apply_model_temperature`/手工 thinking 字段;usage_source/operation 走 `RequestMetadata` 显式传。签名不变。
- [ ] `call_openai_ocr`:同上,user 消息 `[Image{png}, Text{prompt}]`。
- [ ] 检查 `commands.rs:313`、`lens_commands.rs:1520/1586/1719` 编译通过、行为不变。

### 5. 流式入口迁移
- [ ] `call_vision_api`:messages 组装改 `Vec<ModelMessage>`(system 走 `request.system`,thinking-off 的 system 追加指令保留);stream 分支 `stream_with_chat_provider` + LensEventSink,非流式 `generate`。签名不变。
- [ ] `stream_chat_call`:签名 `body: Value` → `system: String, messages: Vec<ModelMessage>`;`lens_commands.rs:1488/1697/1872` 三调用点改组 ModelMessage;`build_ocr_request_body` 删除或改为 ModelMessage 构造器。
- [ ] `stream_translate_combined`:改走 stream_with_chat_provider + combined-splitter sink;取消时旧的 "tail flush 为 translated + emit done(cancelled) + 返回部分文本" 语义保留。
- [ ] 取消路径:捕获 `is_cancelled()` → emit done("cancelled") → Ok(部分文本),不作错误返回。

### 6. 清理 + 全量验证
- [ ] 删除死代码:`stream_vision_response`、`extract_sse_chat_text`、`append_stream_text`、`parse_sse_chat_content`、`build_ocr_request_body`(若已无引用)及相关常量;grep 确认零引用。
- [ ] `send_with_failover_cancelable` 若已无调用方一并删除。
- [ ] `cargo test --manifest-path src-tauri/Cargo.toml`(全量,含集成)。
- [ ] `npm run lint` + `npm run typecheck`(前端无改动,应零噪音通过)。
- [ ] `cargo fmt` 仅对触碰过的文件。

## 验证命令

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib
cargo test --manifest-path src-tauri/Cargo.toml
npm run lint && npm run typecheck
```

## 手动 smoke 清单(实现完成后、报告里列出待用户执行)

- OpenAI 供应商:翻译器 / 截图翻译 / Lens 问答 / Lens 流式 / 流中取消
- Gemini 供应商(`api_format=gemini`):截图翻译(验 404 修复)/ Lens
- 观察 Settings 用量面板:source/operation 维度记录正常、取消记 cancelled

## 回滚点

- 每步一个逻辑单元;整任务单 commit,可整体 revert。
- 步骤 2 之后若发现适配器缺能力(如 vision 编码缺陷),先修适配器再继续,不在 api.rs 内绕。

## 审查门

- 步骤 3 后:sink 单测过再进入 4/5。
- 步骤 6 后:trellis-check 全量核查(重点:事件 payload 形状、取消语义、usage 维度、OpenAI 回归差异仅 max_tokens)。
