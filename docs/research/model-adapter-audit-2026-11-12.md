# 模型适配器审查：2026-11-12

本轮修复 Claude 在 Chat Completions 兼容接口下的思考文本与签名回传，并修复扩展审查发现的 8 项问题。协议解析和请求能力归共享 SDK，桌面层提供供应商配置；费用估算统一归 `usage::estimate_model_cost`。这是调查与验收记录，不是另一份工程规范。

## 范围与依据

- Kivio：共享 SDK 的四种模型协议、桌面模型适配、历史序列化、Agent 用量累加、主聊天与子代理费用，以及模型价格设置。
- PI：固定版本 [`406e719a6bff6dd3639cadecc868ee61048ee4ed`](https://github.com/earendil-works/pi/tree/406e719a6bff6dd3639cadecc868ee61048ee4ed/packages/ai/src)，协议实现位于 `api/`。用官方文档判断协议要求，PI 用于交叉检查；没有照搬其用量口径。
- OMP：核对 Chat Completions 对直接/镜像 `thinking_blocks` 的读取、显示去重与带签名块回传。
- 场景：流式、非流式、多轮文本/思考、函数工具调用、缓存和用量。没有调用真实供应商账户，也没有验证全部模型、多模态生成续轮或 OAuth 传输。

## Claude 思考内容与签名

`reasoning_content` 为空或类型不符时继续读取 `reasoning` / `reasoning_text`；`reasoning_details` 的空 summary 不再遮住 text。直接返回和 `provider_specific_fields` 下的 `thinking_blocks` 都可读取。

存在原生块时不重复显示镜像别名；签名、密文和遮蔽数据不显示为思考文字。签名片段与正文块正确合并，只保存可回传的签名块/遮蔽块。`ChatReasoning` 保存两种 Chat 原生数据：LiteLLM `thinking_blocks` 和 OpenRouter `reasoning_details`。历史保存/恢复后只向同一模型、同一供应商回传，其他协议忽略这些数据。

依据：[LiteLLM 工具续轮](https://docs.litellm.ai/docs/reasoning_content#tool-calling-with-thinking)、[OpenRouter reasoning_details](https://openrouter.ai/docs/guides/best-practices/reasoning-tokens)。服务端不返回可读思考时，客户端不会凭签名生成思考文字。

## 8 项发现及修复

### 1. Gemini 拦截终态被当成成功（已修复）

原行为：`finishReason: SAFETY` 或只有 `promptFeedback.blockReason` 的响应可返回成功；已有函数调用还会覆盖异常终态。

修复：公共解码边界先检查拦截与异常终止原因。安全拦截返回 `ContentModeration`，其他异常保留具体 provider code；仅正常停止可变成工具调用终态，长度截断保留 `Length`。流式/非流式共用规则。

依据：[Google GenerateContent 协议](https://github.com/googleapis/googleapis/blob/master/google/ai/generativelanguage/v1beta/generative_service.proto)，PI `google-shared.ts::mapStopReason`。

### 2. Responses 拒绝文本丢失（已修复）

原行为：拒绝响应可能变成空内容，引发上层空响应重试。

修复：读取 `response.refusal.delta/done`、`output_item.done` 和最终 output 中的 refusal，按消息与内容索引去重；非流式也保留 refusal 文本。

依据：[OpenAI Responses 流事件](https://developers.openai.com/api/reference/resources/responses/streaming-events)，PI `openai-responses-shared.ts`。

### 3. Responses 异常 incomplete 被当成成功（已修复）

原行为：`incomplete_details.reason: content_filter` 返回成功的 Other 终态。

修复：`max_output_tokens` 保留成功的长度截断；content_filter 返回安全拦截，其他 incomplete 返回明确错误。failed/cancelled 不发成功终态，并保留取消分类。

依据：[Responses 状态定义](https://developers.openai.com/api/reference/resources/responses/methods/retrieve)，PI 的结束状态映射。

### 4. Gemini 输出用量漏计思考（已修复）

修复：输出 token = candidates + thoughts，使用饱和加法；reasoning 仍是输出的子集，不重复计费。输入与缓存原有语义不变。更新了原先按 candidates-only 断言的测试。

依据：[Google 思考用量](https://ai.google.dev/gemini-api/docs/generate-content/thinking)，PI `google-generative-ai.ts`。

### 5. 官方 o 系列 Chat 输出上限字段错误（已修复）

修复：已知官方 OpenAI 路由的 o1/o3/o4 模型使用 `max_completion_tokens`，未知兼容网关保留原来的 `max_tokens`。SDK HTTP provider 自动传入 endpoint，桌面原始编码调用传入其供应商 endpoint。单次 provider_options 保持最终覆盖优先级。

依据：[OpenAI Chat 参数](https://developers.openai.com/api/reference/resources/chat/subresources/completions/methods/create)，PI 的 `maxTokensField` 能力判断。

### 6. OpenRouter reasoning_details 在工具续轮丢失（已修复）

修复：保存完整详情与不透明字段，按 index/id 合并可读流片段，密文条目保持独立；snapshot 使用完整数组。经历史恢复后回传原生字段，并校验模型/供应商来源，不伪装成 Responses reasoning item。

验证覆盖流式与非流式、真实本地 HTTP 第二轮请求、历史转换、模型切换及供应商切换。依据：[OpenRouter 工具续轮示例](https://openrouter.ai/docs/guides/best-practices/reasoning-tokens)，PI `appendOpenAIReasoningDetail` 与 `preservedReasoningDetails`。

### 7. 缓存请求没有映射到路由能力（已修复）

- OpenRouter Claude：短期发送自动 `cache_control`，长期带 `ttl:1h`；清除不适用的旧 retention 字段。
- 官方 GPT-5.6 及更新模型：使用 `prompt_cache_options`。关闭缓存设置 explicit mode 且不自动加断点，短期为 30m；长期在 Responses 使用 12h，Chat 使用支持的 30m。
- 旧 OpenAI 模型和未知网关保留原字段，不向 dsivio 等未知路由强塞厂商扩展。模型版本解析区分小版本和快照日期。

依据：[OpenAI 缓存协议](https://developers.openai.com/api/docs/guides/prompt-caching)、[OpenRouter 缓存协议](https://openrouter.ai/docs/guides/best-practices/prompt-caching)，PI `openai-responses.ts::getPromptCacheOptions` 与 Chat cache control 实现。
另外核对了 [OpenRouter 当前 OpenAPI schema](https://openrouter.ai/openapi.json)：Responses 顶层 `cache_control` 引用 `AnthropicCacheControlDirective`，可携带 ttl，因而共享编码规则适用于这一字段。

### 8. 缓存写入费用漏计且存在重复实现（已修复）

修复：主账本和子代理预算共用 `usage::estimate_model_cost`。Chat/Responses 总输入减去读取和写入后计算普通输入；原生 Anthropic 的普通输入本就不含缓存，保持互斥口径。Claude 默认短写价 1.25 倍、长写价 2 倍，新 OpenAI 默认 1.25/2.5 倍；其他模型不继承这些默认倍率。

新增可配置短/长写价，模型价格设置支持独立覆盖；只覆盖短期价格不改变长期默认价格。解析并持久化长时效写入 token，优先使用服务端时效明细；无明细时按请求配置估算。多次调用只有明细完整时才保留合计时效分桶，避免把未知写入误作短期。Chat 的新 OpenAI 请求没有 12h，因此不因本地 long 设置按 12h 估算。

固定测试价格下，普通输入 10、缓存读 80、缓存写 10、输出 5，Claude 两种接口都得到 $0.000111；另测混合时效及自定义写价。数值是配置价格估算，不是供应商实际账单。

依据：[Anthropic 缓存计费](https://platform.claude.com/docs/en/build-with-claude/prompt-caching)、[OpenAI 缓存计费](https://developers.openai.com/api/docs/guides/prompt-caching)，PI `models.ts::calculateCost`。

## 复审结论与边界

四种接口在本次覆盖范围内的已复现缺陷均已实现修复。用量读取另外补齐了 null 详情字段的别名回退；未改变不同协议的输入 token 语义。

Anthropic 的 `input_tokens` 不含缓存读取/写入；Chat 和 Gemini 输入总数含缓存。PI 常把这些数值转换为互斥桶，Kivio 保留协议口径，在上下文展示和费用计算处归一化。Responses store:false 与密文回传、Anthropic 签名/遮蔽块和 pause_turn 续传、Gemini 工具签名来源隔离由现有回归继续覆盖。

截图没有提供 dsivio 原始响应，因而不能认定某个字段形态就是截图的唯一原因。若供应商没有返回缓存计数或可读思考，客户端仍无法还原这些内容。

## 验证记录

- 修复前：临时消费端 9 个协议反例、2 个固定费用断言失败；本轮最先加入仓库的 4 个协议回归也全部先失败，再通过。
- SDK 全套：234 项通过。
- 独立 consumer 全套：最终代码上 161 项通过，包含 8 项 adapter_regressions，覆盖已知/未知路由、两轮 HTTP、历史恢复、TTL 尾帧和异常终态。
- 前端：TypeScript 检查、16 项模型信息合并测试及修改文件 ESLint 通过；架构检查通过。
- 桌面：`cargo check --tests` 通过；通过 Windows 测试脚本构建并运行，用量/费用 27 项、模型适配与元数据 111 项、子代理执行器 22 项通过。旧的未知终态成功断言已改为保留错误码且不重试，Gemini 工具调用的长度截断保持 Length。
- 未执行真实供应商请求、桌面 UI 操作和多模态生成续轮。其他线程的 MessageBubble/CSS 改动不包含在本次提交。
