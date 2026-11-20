# Gemini 内置搜索与函数调用混用修复

## 核验来源

2026-11-21，npm 查询 `@earendil-works/pi-coding-agent` 的 `latest` 为 **1.6.0**，同时核对其发布标签和主线 `597d7d649c88a6cc026dbca59a3e3925f6c5efa5`。

- [Pi 1.6.0 Google 请求构建](https://github.com/badlogic/pi-mono/blob/v1.6.0/packages/ai/src/api/google-generative-ai.ts)：通过 Google SDK 调用 GenerateContent；默认只转换自定义函数，不自动加入 Google Search。
- [Pi 1.6.0 Google 共享转换](https://github.com/badlogic/pi-mono/blob/v1.6.0/packages/ai/src/api/google-shared.ts)：严格工具采样可选 `VALIDATED`；Gemini 3+ 回传函数调用/结果 ID；签名只属于原响应片段，不在片段间挪动，空的签名片段也保留。
- [Google 组合工具说明](https://ai.google.dev/gemini-api/docs/tool-combination)：混用能力支持 Gemini 3 及更新模型；开启工具上下文流转时 `AUTO` 不受支持。当前示例已转向 Interactions API，Kivio 保留既有 GenerateContent 接口，不照搬其请求结构。
- [GenerateContent Part 定义](https://ai.google.dev/api/generate-content#v1beta.Part)：`toolCall`、`toolResponse` 是服务端工具上下文，客户端需要回传；`functionCall`、`functionResponse` 是客户端函数调用及结果，并以 ID 配对。
- [ToolConfig 定义](https://ai.google.dev/api/caching#ToolConfig)：原生 JSON 字段为 `includeServerSideToolInvocations`。错误里的 `include_server_side_tool_invocations` 是同一字段的另一命名形式。
- [Google SDK wire 转换](https://github.com/googleapis/js-genai/blob/main/src/converters/_models_converters.ts)：`config.toolConfig` 转为 REST 顶层 `toolConfig`，并显式透传 `includeServerSideToolInvocations`。SDK 参数对象不能直接当成 REST 请求体。

## 根因与修复归属

原路径：桌面配置 → SDK Gemini 编码器 → 同时发送 `functionDeclarations` 和 `google_search`，仍使用 `AUTO`，缺少服务端工具回传开关；响应解析还丢弃服务端工具块并把兄弟片段的签名借给无签名函数。

新路径保持不变，规则仍唯一归属共享 SDK Gemini 适配器：

- 根据最终工具列表（包括供应商覆盖）识别混用，开启上下文流转，将缺省/`AUTO` 模式设为 `VALIDATED`，保留其他配置字段及显式 `ANY`/`NONE`。显式禁用工具仍最后生效。
- 流式/非流式均保存有序原生片段，服务端工具不作为本地函数执行；删除跨片段签名兜底。历史原样保存和恢复签名、服务端调用/结果，切换模型或已知供应商后剥离原生数据，仅保留可见正文。
- Gemini 3+ 自动回传函数调用/结果 ID，宿主无需另写一套版本规则。已知旧 Gemini 模型在发送前报告不支持混用，未知网关别名交给上游判断。

新增原生消息变体需要历史桥接、其他协议的可见文本投影及上下文估算识别；这些位置只负责契约适配，不另建工具执行或存储负责人。原有 `parameters` schema 转换继续保留，本次不把 Pi 的 JSON Schema 选择扩展到无关的 OAuth 兼容路径。

## 验证

回归先在修改前执行：缺少回传开关、跨片段签名挪用、旧模型缺少本地拦截三项均失败。修复后新增三项通过；其中实际调用本地 HTTP provider，覆盖流式/非流式、有序服务端上下文、双函数 ID、空签名片段、旧历史桥接、JSON 会话快照恢复、第二轮请求，以及跨模型/供应商/协议隔离。

- SDK 全特性库测试：280 项通过。
- 独立消费端全套：164 项通过，包含新增三项回归。
- 架构检查、修改文件 Rust 格式检查、`git diff --check` 通过。
- 桌面 `cargo check --locked --manifest-path src-tauri/Cargo.toml --tests` 通过。使用临时隔离的 target 缓存完成检查，避开其他任务的构建锁；仅有既有宏解析、弃用和未使用导入警告，临时缓存检查后清理。

没有使用真实 Gemini 凭据或执行桌面 UI 验收，不能把本地 HTTP 回归当作真实供应商或实机验证。
