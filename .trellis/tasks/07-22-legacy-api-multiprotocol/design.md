# 设计：翻译/截图/Lens 旧调用路径接入多协议模型适配器

## 现状盘点(事实,已核实)

### 旧路径五个入口(`api.rs`,全部硬编码 `{base_url}/chat/completions` + Bearer)

| 入口 | 行 | 形态 | 调用方 |
|---|---|---|---|
| `call_openai_text` | 658 | 非流式,纯文本 prompt | `commands.rs:313`(翻译器)、`lens_commands.rs:1719` |
| `call_openai_ocr` | 735 | 非流式,带图(base64 image_url) | `lens_commands.rs:1520/1586` |
| `call_vision_api` | 858 | 流式+非流式,自组 system+图+多轮 messages | `lens_commands.rs:979/1270` |
| `stream_chat_call` | 1219 | 流式,**接收调用方预组的 OpenAI body** | `lens_commands.rs:1488/1697/1872` |
| `stream_translate_combined` | 1326 | 流式,`<<<ORIGINAL>>>` 分隔符拆双段 emit | `lens_commands.rs:1553` |

共性基建(保留复用):`send_with_failover(_cancelable)`、`record_api_usage`、`Utf8StreamDecoder`、`explain_stream_generation` 代际取消、事件 payload `{imageId, kind, delta, done, reason}`。

### 聊天侧已有的东西(直接复用,不新造)

- `chat/agent/planning.rs::generate_with_chat_provider` / `stream_with_chat_provider`(`pub(crate)`):按 `provider.api_format_kind()` 分发到四个适配器,签名只要 `&AppState + &ModelProvider + retry + GenerateRequest`,**无 Tauri/AgentHost 依赖**。
- 适配器自带:多 key failover、usage 记录(`RequestMetadata.usage_source/usage_operation` 缺省时才从 label 推断)、temperature 解析(`temperature_for_request`)、thinking 禁用字段(openai.rs:501 与旧路径同款 `thinking:{disabled}`)、`reasoning_content`→`StreamPart::ReasoningDelta`。
- `MessagePart::Image { mime_type, data }` 天然承载 base64 图;各适配器已会按各自协议编码。
- `StreamSink::emit` 返回 `Result<(), ModelError>` —— sink 返回 Err 即中断流,这是适配器层唯一的取消钩子。

## 方案

**一句话:五个入口签名不动(除 `stream_chat_call`),内部改为组 `GenerateRequest` → 调 `planning.rs` 的两个分发函数;SSE 解析删掉,换成一个把 `StreamPart` 翻译成现有 Tauri 事件的 sink。**

### 1. 分发函数提升

`generate_with_chat_provider` / `stream_with_chat_provider` 从 `planning.rs` 移到 `chat/model/mod.rs`(或原地改 `pub(crate)` 导出路径),成为全 crate 的"按 api_format 调模型"唯一入口。不新建 facade 模块——这两个函数就是 facade。

### 2. 事件桥 sink(新增,~80 行,放 `api.rs`)

```rust
struct LensEventSink<'a> {
    app: &'a AppHandle,
    event_name: &'a str,
    image_id: &'a str,
    kind: &'a str,                  // "translated" / "answer" 等,沿用现值
    generation: u64,
    counter: &'a AtomicU64,         // explain_stream_generation
    full: String,
}
impl StreamSink for LensEventSink<'_> {
    fn emit(&mut self, part: StreamPart) -> Result<(), ModelError> {
        if self.counter.load(SeqCst) != self.generation {
            return Err(ModelError::with_kind(CANCEL_SENTINEL, ModelErrorKind::Cancelled));
        }
        match part {
            TextDelta { delta } => { /* emit {imageId, kind, delta} */ }
            ReasoningDelta { delta } => { /* emit kind="reasoning",对齐旧 reasoning_content 行为 */ }
            Finish { .. } => { /* emit done:true */ }
            _ => {} // ToolCall* 不会出现(不传 tools)
        }
        Ok(())
    }
}
```

事件 payload 形状逐字节保持——**UI 契约不动**。

### 3. 取消语义(设计决策)

- 新增 `ModelErrorKind::Cancelled`(types.rs 枚举加一变体)。
- sink 在每次 emit 前检查代际;不匹配则返回 `Cancelled` 错。适配器的 `?` 会把它一路抛回。
- `api.rs` 包装层捕获 `kind==Cancelled`:emit `done("cancelled")`,返回已累积的部分文本(与旧行为一致:取消不是错误)。
- usage 记录状态:四个适配器的 `record_usage_failure` 目前硬编码 `status:"failure"`。改为:错误 kind/消息命中取消哨兵时记 `"cancelled"`。为避免改 4 处签名,在 `usage.rs` 的记录辅助(`error_kind_from_message` 同层)加一个判断即可。
- **放弃**的旧能力:`send_with_failover_cancelable` 的建连阶段取消(适配器用的是普通 `send_with_failover`)。取消将在首个 delta 处生效,最坏延迟 = 建连+首包时间。不值得为此给四个适配器加取消闭包参数;若日后需要,在 `GenerateRequest` 加 `cancel: Option<Arc<AtomicBool>>` 一处解决。

**实现后修订(usage 行为):** 中途取消(流已开始后代际失配)时 sink 返回的 `Cancelled` 错经适配器 `sink.emit(...)?` 裸抛回,**不经过** `record_usage_failure`——因此不再写 usage 行。旧路径此处会写一条 `status="cancelled"`、`usage:None`(无 token 数据)的标记行,现取消该行为。**决策:接受**——(1) 无 token 数据丢失,仅少一条"发生过取消"标记;(2) 与 chat 路径语义统一(chat 的 AgentStreamSink 取消同样不记 usage);(3) 修复它需碰 chat 共享路径(设计明令不改)或在 api.rs 重新引入自管 usage(本任务要消除的)。`failure_status_from_message` 仍有用:**建连阶段**取消(`send_with_failover` 返回 cancelled 错)会流经 `record_usage_failure`,此时正确记 `"cancelled"`(旧版记 `"error"`),chat/legacy 两路径同时受益。

### 4. 各入口改造

- **`call_openai_text`**:签名不变。body 组装 → `GenerateRequest { system:"", messages:[user text], options:{thinking_enabled, ..Default}, metadata:{usage_source/operation} }` → `generate_with_chat_provider` → 取 `output.text`。旧的 `apply_model_temperature`/`thinking` 手工字段删除(适配器已覆盖同语义)。
- **`call_openai_ocr`**:同上,user message 为 `[Image{png base64}, Text{prompt}]`。
- **`call_vision_api`**:message 组装从 raw JSON 改为 `Vec<ModelMessage>`(system 单独走 `request.system`);stream 分支用 `LensEventSink`,非流式走 `generate`。签名不变。
- **`stream_chat_call`**:**签名改**——`body: Value` 参数换成 `system: String, messages: Vec<ModelMessage>`。调用方 `lens_commands.rs` 三处从 `build_ocr_request_body`(组 OpenAI JSON)改为组 `ModelMessage`;`build_ocr_request_body` 随迁移删除或缩为 ModelMessage 构造器。
- **`stream_translate_combined`**:分隔符拆分状态机(tail 缓冲、UTF-8 边界)**原样保留**,只是输入从"自己解析 SSE 的 delta"换成"sink 收到的 TextDelta"。实现为一个包装 sink:内部持有拆分状态,把 TextDelta 按分隔符切成 translated/original 两种 kind 再 emit。
- 旧 SSE 解析(`stream_vision_response`、`extract_sse_chat_text`、`parse_sse_chat_content` 等)在所有调用方迁完后删除。

### 5. 行为对齐点(潜在差异,逐条决策)

| 项 | 旧行为 | 适配器行为 | 决策 |
|---|---|---|---|
| max_tokens | 不发送 | `GenerateOptions` 默认 8192 | 翻译/OCR 场景 8192 足够;沿用默认,不加知设置 |
| temperature | `apply_model_temperature` | `temperature_for_request` | 同一元数据源,等价;删旧调用 |
| thinking 禁用 | body 手工 `thinking:{disabled}` + system 追加禁止指令 | openai 适配器同款字段;system 追加逻辑在 `call_vision_api` 保留 | 等价 |
| usage 维度 | `record_api_usage(source, operation)` | `RequestMetadata.usage_source/operation` 显式传入 | 显式传,不靠 label 推断 |
| 失败重试 | `send_with_retry` + failover | 适配器内同一套 | 等价 |
| HTTP 非 2xx snippet | 截 500 字符入错误 | 适配器各自的错误报文 | 接受措辞变化(仅错误文案) |

### 6. 兼容与回滚

- OpenAI 供应商:同协议、同 URL、同鉴权,唯一 body 差异是显式 `max_tokens` —— 回归风险集中在这一点,smoke 覆盖。
- 回滚:改动集中在 `api.rs` + `lens_commands.rs` 调用点 + types.rs 一个枚举变体,单 commit 可整体 revert。
- 不改:`chat/` 的聊天路径、四个适配器的 wire 细节、前端任何代码。

## 测试策略

- Rust 单测:`LensEventSink` 的取消/事件序列;combined-splitter 包装 sink 的分隔符跨 delta、UTF-8 边界 case(把现有注释里的两个关键点变成测试)。
- 现有 `loop_tests.rs` 不受影响(分发函数移动是纯搬迁)。
- 手动 smoke(无 e2e):OpenAI 供应商 × {翻译器、截图翻译、Lens 问答、Lens 流式、取消};Gemini 供应商 × {截图翻译、Lens};有条件时 Anthropic 供应商抽查一项。

## 工作量与顺序

1. types.rs 加 `Cancelled` 变体 + usage 记录判断(小)
2. 分发函数搬到 `chat/model/mod.rs`(纯移动)
3. `LensEventSink` + combined-splitter sink + 单测
4. `call_openai_text` / `call_openai_ocr`(非流式,先易)
5. `call_vision_api` / `stream_chat_call`(+ lens_commands 调用点)/ `stream_translate_combined`
6. 删除死代码(旧 SSE 解析)、全量测试、smoke
