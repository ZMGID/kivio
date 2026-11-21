# Kivio Agent 模型请求适配审查 · 2026-11-21

## 复现与改动

桌面端本地 HTTP 测试复现了一个与供应商无关的错误。调用方要求一次性返回，模型配置里的 `stream: true` 却改变了实际出站请求，接收方仍按一次性结果处理。另一个问题是同一份模型设置换到其他协议后，额外请求体字段会丢失。现在五种桌面接口都读取模型配置，最终的模型标识和传输方式由实际适配器确定。回归测试从桌面调用入口发出请求，检查服务器收到的 URL、JSON 以及调用方收到的文本。

| 接口 | 修复与取舍 |
| --- | --- |
| OpenAI Chat Completions / Responses | 官方模型按实际工具、历史和推理档位选择可用端点。切换时转换函数声明、指定工具、结构化输出与 token 上限；无法转换的 Chat 参数会在联网前给出字段名。官方模型在合并自定义参数后再次检查采样和推理限制。Responses 续轮保留分段的助手输出，并为重建的消息生成稳定 ID 和完成状态；保留可选工具参数，并对仅支持普通 HTTP 返回的 o1-pro、o3-pro 使用一次性响应解码。 |
| Anthropic Messages | 桌面与独立 SDK 共享 Claude 能力规则，分别处理手动推理预算、自适应推理、关闭思考及模型允许的采样参数。手动预算与输出上限一起校验，按最终请求选择 beta 请求头，历史工具调用始终提供对象类型的 input。 |
| Gemini GenerateContent | 原生函数声明使用 parametersJsonSchema，保留工具定义的引用、可空类型与属性名。Antigravity 的 Claude / GPT-OSS 路由沿用其兼容格式。SDK 和桌面按模型选择思考预算、档位或摘要，覆盖不接受对应参数的图像模型。 |
| xAI Responses | 用 Grok 支持的档位表达客户端的思考设置。按最终可用的原生工具请求来源等附加输出。Grok 4.7 的后续工具轮可回传来自同一供应商、端点和模型的加密推理项。 |
| DeepSeek Responses | 官方 Chat 配置启用原生搜索时，转换 Chat 请求字段再交给 Responses。解析 JSON 或 SSE 中的明文推理，在同源工具续轮中恢复；屏蔽自动生成的加密推理及缓存选项。历史助手图片以文字占位，用户图片照常发送。 |

新保存的 Responses 推理项记录来源协议、供应商、端点和模型。端点记录排除认证信息、查询参数和片段。旧历史保持可读，原有 OpenAI 模型名匹配路径保留；新的 xAI 回放要求有来源。历史记录未保存 Responses 的原始助手消息 ID 和 phase，续轮使用确定性 ID，省略未知的 phase。直接在 Rust 中构造推理项的 SDK 调用方需要填写新增的可选 `source` 字段。

## 对照资料与复核

- Pi 固定到 v1.6.0，核对了 Chat 与 Responses 的消息和工具映射、流事件处理及 Claude、Gemini 适配。对照 Pi 的流式实现后，依据官方模型文档为 o1-pro 和 o3-pro 增加了非流式交付路径。[Pi 适配器源码](https://github.com/earendil-works/pi/tree/v1.6.0/packages/ai/src/api) · [o1-pro](https://developers.openai.com/api/docs/models/o1-pro) · [o3-pro](https://developers.openai.com/api/docs/models/o3-pro)
- OpenAI 通过模型指南、函数工具文档和 SDK 上游实际返回的 400 核对请求。复核纠正了助手文本块类型，并补上了原始参数覆盖后重新引入不支持的采样参数、Chat 独有的流选项进入 Responses 等回归。[模型指南](https://developers.openai.com/api/docs/guides/latest-model) · [函数工具](https://developers.openai.com/api/docs/guides/function-calling) · [Responses 请求参考](https://developers.openai.com/api/reference/resources/responses/methods/create) · [SDK 上游问题和错误样例](https://github.com/openai/openai-python/issues/3544)
- Anthropic 与 Google 对照官方 SDK 类型和序列化器。独立审查发现了 Claude 关闭思考的模型差异、独立 SDK 的 Gemini 默认档位，以及跨协议转换时较低优先级的配置遮住本次调用的问题，均补入回归。[Claude 思考文档](https://platform.claude.com/docs/en/build-with-claude/thinking) · [Anthropic SDK](https://github.com/anthropics/anthropic-sdk-typescript/blob/main/src/resources/messages/messages.ts) · [Gemini 思考文档](https://ai.google.dev/gemini-api/docs/thinking) · [Google Gen AI SDK](https://github.com/googleapis/js-genai/blob/main/src/converters/_models_converters.ts)
- xAI 和 DeepSeek 都提供 Responses 协议，接受的思考参数及历史数据不同。各自的处理依据官方 API 文档；第三方端点继续接收其自定义字段。[Grok 4.7](https://docs.x.ai/developers/grok-4-7) · [DeepSeek Responses](https://api-docs.deepseek.com/guides/responses_api/)

## 验证记录

- 独立 SDK 启用全部功能后，306 项测试通过。独立消费端 180 项测试通过，其中包含真实本地 HTTP 服务检验请求体、两轮工具调用、流事件、错误恢复和来源隔离。
- 前端设置、模型详情和内置搜索的 32 项测试通过。TypeScript、相关 ESLint、Rust 格式、SDK 依赖边界和项目架构检查通过。
- 桌面端模型、缓存与重试、Antigravity 适配及 Agent 循环共 240 项通过，包括历史压缩后继续对话、保存并重载后继续调用。两个 DeepSeek 专项联网压力或子代理场景按仓库配置跳过，临时的真实接口探针单独运行，结果见下表。
- Rust 套件可用以下命令重跑。桌面端本次使用隔离工作树运行，避免并行开发中其他模块的测试编译冲突，模型实现与主工作树逐文件比对。

```sh
cargo test --locked --manifest-path crates/kivio-agent-sdk/Cargo.toml --all-features
cargo test --locked --manifest-path tests/fixtures/agent-sdk-consumer/Cargo.toml
cargo test --locked --manifest-path src-tauri/Cargo.toml --lib -- 'chat::model::' 'chat::model_metadata::' 'provider_oauth::antigravity::' 'learned_provider_capabilities_are_endpoint_scoped_and_independent' 'chat::agent::loop_::tests::'
```

### 真实接口

使用已启用的 Kivio 供应商配置，通过桌面模型调用入口发送合成消息。第一轮流式调用本地测试函数，第二轮回传函数结果并校验最终文本含测试词。每条成功线路都实际走了这两轮。

| 接口 | 实测结果 |
| --- | --- |
| 智谱官方 Chat，glm-5.3-flash | 通过 |
| Google 官方 Gemini，gemini-3.1-flash-lite | 通过 |
| DeepSeek 官方 Responses，deepseek-flash | 通过 |
| 兼容网关 Responses，gpt-5.6-sol | 通过 |
| 兼容网关 Anthropic Messages，claude-sonnet-4-6 | 403，账号分组无权限 |
| 兼容网关 Responses，gpt-6-luna，以及另一条 Chat 线路的 glm-5.3-flash | 503，网关无可用模型渠道 |

OpenAI 官方、Anthropic 官方、xAI 官方及 Antigravity OAuth 的真实服务未在本次调用中验证。上述线路的本地测试覆盖请求格式或适配逻辑。Gemini 和 Responses 的真实调用也未覆盖所有图片输入、原生搜索和长时间运行场景。

## 第二轮复核

- **Responses 工具调用的完成边界。** 本地 HTTP 用例复现两种相反的错误：只收到参数增量时，外层完成事件会把未确认的调用交给 Agent；完整调用只出现在最后的输出快照时，Agent 又会漏掉它。现在按工具项标识合并流事件与最终快照，每个已确认的调用只交付一次。普通 JSON 响应也检查工具项的完成状态。触达输出长度上限时，保留原有截断标记，由 Agent 拒绝执行这一批工具。[Pi 流处理](https://github.com/earendil-works/pi/blob/v1.6.0/packages/ai/src/api/openai-responses-shared.ts) · [OpenAI 流事件参考](https://developers.openai.com/api/reference/resources/responses/streaming-events)
- **Responses 最终正文。** 本地 HTTP 用例分别发送完整增量、部分增量和仅含最终输出的 SSE。最终快照的可见正文可以包含 output_text 和 refusal。现在当快照与已发送的文字前缀吻合时，适配器将剩余后缀交给流式界面，并在最终输出中保留完整文字。[OpenAI 流事件参考](https://developers.openai.com/api/reference/resources/responses/streaming-events)
- **Gemini 工具失败。** 在包含拒绝授权、参数校验失败和成功执行的真实 SDK 会话循环测试中，失败结果原先也会以普通输出回传。现在通过 functionResponse.response.error 告知模型失败原因，保留调用 ID、函数名及成功结果原有的 JSON 结构。[Google FunctionResponse 协议定义](https://github.com/googleapis/googleapis/blob/master/google/ai/generativelanguage/v1beta/content.proto) · [Pi 的 Gemini 适配](https://github.com/earendil-works/pi/blob/v1.6.0/packages/ai/src/api/google-shared.ts)
- **Claude 缓存断点。** 已包含四个用户断点的合法请求，过去仍会被追加默认断点；不同缓存时长也可能被覆盖或排错顺序。现在显式断点或顶层缓存设置优先，长缓存兼容头根据最终请求中实际出现的时长生成。没有显式配置时继续使用 Kivio 的默认断点布局。[Anthropic 缓存文档](https://platform.claude.com/docs/en/build-with-claude/prompt-caching)

第二轮验证通过 SDK 307 项、独立消费端 183 项、桌面适配器及 Agent 循环 240 项。桌面套件在独立工作树执行，两项需专门配置的 DeepSeek 联网测试按仓库设置跳过。本轮新增场景使用本地 HTTP 服务和请求体断言复现。
