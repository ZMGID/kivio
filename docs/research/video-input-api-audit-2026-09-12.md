# 视频输入：官方 API 格式与 Kivio 接入核查

调查日期：2026-09-12。代码基线：`88b8332e`，另含上一轮尚未提交的模型能力修补。

## 判定标准

本报告的“支持”指指定供应商的指定接口可以接收视频并进行理解，不指视频生成，也不把开源权重的视觉能力等同于托管 API 能力。官方文档证明协议契约；没有执行付费模型请求，故不把文档核查写成端到端实测通过。任意中转站、私有模型别名和账号权限仍需单独确认。

结论：不能再用一个 `videoInput: true` 加 `OpenAI-compatible` 推导完整接入。应同时确认模型 ID、供应商路由、API 格式、输入载体与限制。以下请求片段是最小结构示意，`BASE64_VIDEO`、文件 ID 均为占位符，不是可发送的完整视频。

## 已有接入：Kimi

### 明确支持的官方 ID

当前官方视觉指南明确列出 `kimi-k3`、`kimi-k2.6`、`kimi-k2.7-code`、`kimi-k2.7-code-highspeed` 的视频理解能力。[Kimi 视觉指南](https://platform.kimi.ai/docs/guide/use-kimi-vision-model)

无后缀的 `kimi-k2.7` 不在本次核查的官方在售模型表中。不能因为 Code 版支持就确认该别名在任意供应商也支持。`kimi-k2.5` 有历史能力，但官方已于 2026-08-31 停用，不能作为当前官方接口推荐。[Kimi 模型表](https://platform.kimi.ai/docs/models)

Kimi Code 的 `k3`、`kimi-for-coding` 等订阅路由与 Moonshot API Key 路由是不同入口；本次不将上述 API 能力自动扩展到订阅、OAuth 或 Anthropic 兼容入口。

### Chat Completions

`POST https://api.moonshot.ai/v1/chat/completions`；Bearer API Key。视频在用户消息的 `content` 数组内，不得把数组 JSON 序列化成文本；`video_url.url` 支持 data URL 和文件引用。如下对象形式与当前 Kivio 的 Chat 序列化一致。[Kimi Chat API](https://platform.kimi.ai/docs/api/chat)

```json
{
  "model": "kimi-k2.7-code",
  "messages": [{
    "role": "user",
    "content": [
      {"type": "video_url", "video_url": {"url": "data:video/mp4;base64,BASE64_VIDEO"}},
      {"type": "text", "text": "描述视频中发生的事情。"}
    ]
  }]
}
```

官方 K2.7 Code 示例直接使用 Base64 `video_url`；列出的容器格式包括 mp4、mpeg、mov、avi、x-flv、mpg、webm、wmv、3gpp。高分辨率不等于更好，官方建议视频不超过 FHD。文档将大视频和重复引用引导到 Files API；这里不把其图像段落的“100M”描述直接当成所有视频的明确大小承诺。[K2.7 Code 指南](https://platform.kimi.ai/docs/guide/kimi-k2-7-code-quickstart)

K3 官方示例上传 `purpose="video"`，随后使用 `{"type":"video_url","video_url":{"url":"ms://FILE_ID"}}`，最后删除上传文件。Kivio 尚未实现这套文件生命周期；普通 HTTP URL 也不能仅凭字段名 `video_url` 就判定可用。[K3 视频示例](https://platform.kimi.ai/docs/guide/kimi-k3-quickstart)

### 获取模型

官方 `GET /v1/models` 返回 `data[].supports_video_in`。这是对具体 ID 的供应商声明，应优先于静态库，但不能覆盖用户明确关闭的能力。Kivio 已读取此字段；普通兼容站如果只返回 ID，则仍依赖本地数据库。[Kimi List Models](https://platform.kimi.ai/docs/api/list-models)

## 已有接入：Gemini

### 模型与接口必须分开

本次逐页确认 `gemini-3.8-flash`、`gemini-3.1-pro-preview`（页面另列 `-customtools` 变体）、`gemini-2.5-flash` 的输入模态包含视频。不能据此给 embedding、image-generation、TTS、Live 或任意 Gemini 私有别名一概打开视频。[3.8 Flash](https://ai.google.dev/gemini-api/docs/models/gemini-3.8-flash)、[3.1 Pro Preview](https://ai.google.dev/gemini-api/docs/models/gemini-3.1-pro-preview)、[2.5 Flash](https://ai.google.dev/gemini-api/docs/models/gemini-2.5-flash)

`gemini-3-pro-preview` 官方已在 2026-03-09 下线。上一轮补的是历史 ID 的能力识别，不是恢复接口可用性，不应作为新导入的推荐模型。[旧模型卡](https://ai.google.dev/gemini-api/docs/models/gemini-3-pro-preview)

### Kivio 使用的 Generate Content 格式

另逐页确认 `gemini-3.7-flash`、`gemini-3.6-flash`、`gemini-3.5-flash` 均列 Video 输入。[3.7](https://ai.google.dev/gemini-api/docs/models/gemini-3.7-flash)、[3.6](https://ai.google.dev/gemini-api/docs/models/gemini-3.6-flash)、[3.5](https://ai.google.dev/gemini-api/docs/models/gemini-3.5-flash)。`gemini-3.2-pro` 的目标模型卡本轮未成功读取，因此不把其既有标记列入本轮已逐页验证清单。

`POST /v1beta/models/{model}:generateContent`；流式使用 `:streamGenerateContent`；API Key 通过 `x-goog-api-key`。媒体不是 OpenAI 的 `video_url`，而是 `contents[].parts[].inlineData`；`data` 是裸 Base64，不能包含 `data:video/...;base64,` 前缀。上传文件则是独立的 `fileData` 分支。`videoMetadata` 是 Part 同级的可选元数据，不能塞进 Blob。[Generate Content API：Part / Blob / VideoMetadata](https://ai.google.dev/api/generate-content)

```json
{
  "contents": [{
    "role": "user",
    "parts": [
      {"inlineData": {"mimeType": "video/mp4", "data": "BASE64_VIDEO"}},
      {"text": "描述视频中发生的事情。"}
    ]
  }]
}
```

该接口的视频指南要求内联请求总量小于 20 MB；大文件应上传后通过 `fileData.fileUri` 引用。当前对应页面的 MOV MIME 是 `video/quicktime`。此处应按实际调用的 Generate Content 文档实施，不把其他页面、其他接口的限制混进来。[Generate Content 视频指南](https://ai.google.dev/gemini-api/docs/generate-content/video-understanding)

### Interactions 是另一套契约

新版指南使用 `POST /v1beta/interactions`，形如 `input:[{"type":"video","uri":"FILE_URI","mime_type":"video/mp4"}]`；内联形式、处理选项和大小说明应按该接口独立核查。不能将这里的 `processing`、`uri` 或页面顶部的 100 MB 内联说明直接搬到 Kivio 的 `generateContent` 请求。Kivio 尚无此适配。[Interactions 视频指南](https://ai.google.dev/gemini-api/docs/video-understanding)

### Google 的 OpenAI 兼容入口

本次官方兼容文档中未找到 Chat 视频输入的 `video_url` 契约；页面的 `videos.create` 是视频生成，不是视频理解。不能因模型是 Gemini，就让 Google `/v1beta/openai/chat/completions` 自动走 Kimi 格式。若经过 OpenRouter 等中转，须以该中转自己的视频输入文档为准。[Google OpenAI compatibility](https://ai.google.dev/gemini-api/docs/openai)

## 其他供应商核查总览

视频支持必须由 **供应商 + 接口协议 + 确切模型 ID + 传入方式** 共同决定。`vision=true`、名字包含 VL、OpenAI-compatible 都不足以证明可发送 `video_url`。尤其不能因某个供应商支持扩展字段就发给 OpenAI 或 Anthropic 本身。

| 供应商 | 核查结果 | 当前本地视频转 data URL 路线 |
| --- | --- | --- |
| 阿里云 Model Studio | 多个具体模型官方确认视频输入，Chat 用 `video_url` | 字段可复用，但必须有更小的 Base64 上限 |
| MiniMax | `MiniMax-M3` 确认；M2/M2.1/M2.5/M2.7 明确无图像/视频 | Chat 可接；Anthropic 兼容需独立 `video` 适配 |
| 智谱 BigModel | `glm-5v-turbo`、`glm-4.6v`、`glm-4.6v-flash`、`glm-4.5v` 有视频 URL 示例 | 直连 Base64 视频未核实，不能据图片 Base64 示例启用 |
| 火山方舟 | 第一方 SDK 确认 Chat/Responses 的视频结构；第一方项目列举 Seed 2.0 视频模型 | Chat 字段可接，但具体模型/限制仍须按方舟文档补齐 |
| OpenRouter | API 明确支持视频，实际路由能力有差异 | Chat data URL 可接；必须核对 input_modalities 与上游 |
| 腾讯 TokenHub | 明确视频 API，但部分模型仅 URL | 不应给全部视觉模型启用本地 data URL |
| OpenAI | GPT-5.6 Sol/Terra/Luna、GPT-6 Astra 明确 Video 不支持 | 禁止因 Chat 兼容推断视频；其他具体新模型另查 |
| Anthropic | 已查 Vision 仅图片块；无可据以实施原生视频块的证据 | 不启用，文件/代码工具处理视频另论 |
| xAI | Chat 文档为 text/image，搜索工具能看 X 视频 | 没有证明可发本地 `video_url`，不启用 |
| Amazon Nova | `us.amazon.nova-lite-v1:0` 官方视频例子明确 | Bedrock 原生 video/source，需要独立适配 |

## 阿里云 Model Studio / DashScope

官方视觉模型页明确列出以下视频模型 ID：`qwen3.8-max`、`qwen3.8-flash`、`qwen3.7-plus`、`qwen3.7-plus-2026-05-26`、`qwen3.7-flash`、`qwen3.7-flash-2026-07-15`、`qwen3.7-max-2026-06-08`、`qwen3.6-plus`、`qwen3.6-plus-2026-04-02`、`qwen3.6-flash`、`qwen3.6-flash-2026-04-16`、`qwen3.6-35b-a3b`、`qwen3.5-plus`、`qwen3.5-plus-2026-02-15`、`qwen3.5-flash`、`qwen3.5-flash-2026-02-23`、`qwen3.5-397b-a17b`、`qwen3.5-122b-a10b`、`qwen3.5-27b`、`qwen3.5-35b-a3b`、`qwen3-vl-plus`、`qwen3-vl-flash`。同页列出 `qwen3.5-omni-plus` / `qwen3.5-omni-flash` 的视频能力，但 Omni 应另核其专门接口的流式/模态要求，不直接套普通 VL 完整流程。地区可用性要以对应地区模型广场为准。[官方模型页](https://www.alibabacloud.com/help/tc/model-studio/vision-model/)

兼容 Chat 端点 `/compatible-mode/v1/chat/completions` 的内容片段如下；`fps` 是片段顶层字段，**不是** `video_url` 的子字段。公网 URL 可替换 data URL。DashScope 原生内容片段则是 `{"video":"...","fps":2}`，不能混用。[官方视频调用、Base64 示例与限制](https://www.alibabacloud.com/help/en/model-studio/vision)

```json
{"type":"video_url","video_url":{"url":"data:video/mp4;base64,<BASE64>"},"fps":2}
```

同一官方指南规定：Base64 **编码后的字符串 <10MB**；不是原文件 10MB，更不是统一 14MiB 原文件。公网 URL 的大小上限按模型为 2GB/1GB/150MB；SDK 本地路径方式为原文件 100MB，不能把该限制移到 HTTP data URL。Qwen3.5/3.6/3.7/3.8 的视频时长 2秒至2小时；`qwen3-vl-plus`/`qwen3-vl-flash` 为 2秒至1小时。`fps` 默认2、范围0.1–10。文档明确视频以抽帧序列理解，不能据此宣称处理原声。当前适配建议只先开放上述确认 ID 的兼容 Chat 路径、支持格式与编码大小校验；不要通配所有 `qwen*`。[官方限制](https://www.alibabacloud.com/help/en/model-studio/vision)

## MiniMax

接入仍有一个细节必须补证：下方已读示例用的是 `mm_file://`，正文虽然确认 Base64，但没有在该示例中展示 Base64 的完整前缀。真正实现本地字节路径前，应核对官方 schema 或具体 Base64 示例；不能把“支持 Base64”自动当成已经验证 Kimi data URL 的逐字兼容。

`MiniMax-M3` 的托管 OpenAI Chat 已明确支持图片和视频，端点 `https://api.minimax.io/v1/chat/completions`。视频片段 `type=video_url`，`video_url.url` 可为 URL、Base64 视频或 `mm_file://{file_id}`；官方实例给出了嵌套 `detail`。视频支持 MP4/AVI/MOV/MKV，URL/Base64 视频50MB，请求体64MB；Files视频512MB。`fps` 默认1、范围0.2–5，但本轮可读 SDK 指南未展示其确切嵌套位置，因此不要凭 Qwen/豆包格式补 `fps`。M3仅输入视频并不代表支持音频输入。[官方 OpenAI SDK 指南](https://platform.minimax.io/docs/api-reference/text-openai-api)

```json
{"type":"video_url","video_url":{"url":"mm_file://<FILE_ID>","detail":"default"}}
```

MiniMax Anthropic 兼容端点为 `/anthropic/v1/messages`，M3 扩展使用 `type=video`；它支持 URL/Base64/mm_file 来源，不能直接发送 Chat 的 `video_url` 块。M2、M2.1、M2.5、M2.7（包括对应 highspeed）明确仅支持文本与工具块，无图片/视频。此次没有充分核对到 Anthropic `video.source` 的完整示例，实施前应取得官方 schema，而不能凭标准图片块猜造。[官方 Anthropic 兼容说明](https://platform.minimax.io/docs/api-reference/text-anthropic-api)

## 智谱 BigModel

最新 Chat schema 另明确给出 `glm-5.3-flash` 视频 URL 示例，并对 GLM-5.3-Flash、GLM-5V-Turbo、GLM-4.6V、GLM-4.5V 写明视频200M以内、MP4/MKV/MOV。该字段仍定义为视频URL，没有据此确认新型号的本地Base64载体。[官方最新 Chat schema](https://docs.bigmodel.cn/api-reference/模型-api/对话补全.md)

这些确切模型在官方托管 API 文档有 `video_url` 示例：[`glm-5v-turbo`](https://docs.bigmodel.cn/cn/guide/models/vlm/glm-5v-turbo)、[`glm-4.6v`](https://docs.bigmodel.cn/cn/guide/models/vlm/glm-4.6v)、[`glm-4.6v-flash`](https://docs.bigmodel.cn/cn/guide/models/free/glm-4.6v-flash)、[`glm-4.5v`](https://docs.bigmodel.cn/cn/guide/models/vlm/glm-4.5v)。4.6V 页面另列 FlashX 版本，但本轮未单独验证它的完整 model ID 与调用示例，因此只列为待确认。

```json
{"model":"glm-4.6v","messages":[{"role":"user","content":[{"type":"video_url","video_url":{"url":"https://cdn.bigmodel.cn/agent-demos/lark/113123.mov"}},{"type":"text","text":"描述视频"}]}]}
```

上例端点 `https://open.bigmodel.cn/api/paas/v4/chat/completions`，来源为上述 GLM-4.6V 官方页。页面明言不能同时理解文件、视频、图像。该页 Base64 示例是 **图片**；本轮没有查到可以证明 **视频 data URL** 可直接发送的说明。故目前可以确认公网 URL 视频，不能把原文件转 Base64 后直接假定兼容。所谓“约一小时视频”是上下文能力说明，不等于所有视频编码、大小、数量的接口承诺。[GLM-4.6V 官方说明](https://docs.bigmodel.cn/cn/guide/models/vlm/glm-4.6v)

旧型号 `glm-4v-plus-0111` 的视频能力、视频<200M有官方 FAQ 说明；`glm-4v-flash` 是单图模型。这个200M不能套给新型号。[官方 FAQ](https://docs.bigmodel.cn/cn/faq/api-issues)

进一步核查发现，`glm-4v-plus-0111` 自身官方文档有视频裸 Base64 示例：`video_url.url` 直接接编码字符串，没有 `data:video/...;base64,` 前缀。最新 schema 还要求该旧型号视频块放在 `content` 第一项。这是独立适配路径，不能把 Kimi 的 data URL 原样复用，也不能据旧型号推断新型号。[旧型号官方示例](https://docs.bigmodel.cn/cn/guide/models/vlm/glm-4v-plus-0111)、[顺序要求](https://docs.bigmodel.cn/api-reference/模型-api/对话补全.md)

## 火山方舟 / Doubao

第一方 SDK 明确 Chat 结构为下面形式，**fps 在 video_url 内部**，与阿里云不同；url 注释接受视频URL或Base64数据。[官方 Python SDK Chat 类型](https://github.com/volcengine/volcengine-python-sdk/blob/master/volcenginesdkarkruntime/types/chat/chat_completion_content_part_video_param.py)

```json
{"type":"video_url","video_url":{"url":"<VIDEO_URL_OR_BASE64>","fps":1}}
```

Responses 则为以下结构；SDK明确 video_url 可以是完整 URL 或 Base64 data URL，另外可传 `file_id`，但不要把互斥关系、上传purpose与有效期凭空补上。[官方 Responses 类型](https://github.com/volcengine/volcengine-python-sdk/blob/master/volcenginesdkarkruntime/types/responses/response_input_video_param.py)

```json
{"type":"input_video","video_url":"data:video/mp4;base64,<BASE64>","fps":1}
```

火山第一方 OpenViking 配置指南明确推荐 `doubao-seed-2-0-lite-260428`、`doubao-seed-2-0-mini-260428` 用于音视频理解，并说明 `ep-*` 需核查背后基础模型。其当前实现支持 MP4/AVI/MOV，而解析器可接收更多格式不代表 API 能理解所有格式。[官方 OpenViking 配置](https://github.com/volcengine/OpenViking/blob/main/docs/zh/guides/01-configuration.md)

本轮直接打开方舟[视觉文档](https://www.volcengine.com/docs/82379/1362931)仅得到 JavaScript 空壳。LAS官方算子确认 `doubao-1.5-vision-pro-32k` 视频能力，但它的包装层会构造消息/预签名，不能把算子输入字段当Chat接口规范。[官方 LAS 算子](https://www.volcengine.com/docs/6492/2165095?lang=zh) 因此 `doubao-seed-2-0-pro-260215`、Seed1.6/1.8 的精确直连视频白名单与硬限制在本笔记仍标未完成；不使用转售站文档补齐。

## OpenRouter

`/api/v1/chat/completions` 使用 `{"type":"video_url","video_url":{"url":"<URL_OR_DATA_URL>"}}`；`/api/v1/responses` 使用 `input_video`；`/api/v1/messages` 明确不支持视频。必须检查模型 `input_modalities` 包含 `video`，不能只看 `image`。Google AI Studio 路由只支持 YouTube URL；Vertex路由不支持视频 URL，应使用Base64 data URL。这是 OpenRouter 路由约束，不能移作 Google 原生 API 的全部约束。文档列 MP4/MPEG/MOV/WebM；具体时长/大小还须核对上游。本文未调用最新模型列表 API，故未把可能变化的 OpenRouter 模型 ID 批量加入白名单。[官方 Video Inputs](https://openrouter.ai/docs/guides/overview/multimodal/videos)

## 腾讯 TokenHub

`https://tokenhub.tencentmaas.com/v1/chat/completions` 接受同形 `video_url`。官方已确认 `hunyuan-turbos-vision-video-20250728` 用公网URL；**HY-Vision-Video、YT-VITA、GLM-5V-Turbo 不支持Base64**。`kimi-k3` 有 data URL 与多视频示例；多视频另确认 Kimi K2.6、K2.7系列和Qwen3.5-Flash。平台单视频与请求体上限均100MB，仍受模型侧限制。不能把这里GLM的URL-only结论推给BigModel直连，但也不能给TokenHub中的所有视频模型开放本地Base64选择器。[腾讯官方视频文档](https://cloud.tencent.com/document/product/1823/136957)

## OpenAI / Anthropic / xAI：不要误启用

当前型号也逐页核过：`gpt-5.6-sol`（`gpt-5.6` 对应页面）、`gpt-5.6-terra`、`gpt-5.6-luna`、`gpt-6-astra` 都明确列 Video 不支持，而不是只用旧型号作推断。[Sol](https://developers.openai.com/api/docs/models/gpt-5.6-sol)、[Terra](https://developers.openai.com/api/docs/models/gpt-5.6-terra)、[Luna](https://developers.openai.com/api/docs/models/gpt-5.6-luna)、[Astra](https://developers.openai.com/api/docs/models/gpt-6-astra)

- OpenAI `gpt-5.4`、`gpt-5.4-2026-03-05` 的模型页明确列 Video 不支持。[官方模型页](https://developers.openai.com/api/docs/models/gpt-5.4) Sora2的Video为输出，不是此处视频问答输入。[官方 Sora2 页](https://developers.openai.com/api/docs/models/sora-2) 此处没有完成所有更新型号审计，不泛化成“OpenAI永远不支持视频”。
- Anthropic Vision 文档用 `image`+`source`，支持JPEG/PNG/GIF/WebP且动画只取第一帧；未见原生video内容块证据。[官方 Vision](https://platform.claude.com/docs/en/build-with-claude/vision) Files/代码执行能保存、处理或产生MP4不等于Messages原生视频理解。[官方 Files](https://platform.claude.com/docs/en/build-with-claude/files)
- xAI Chat 官方接口定义为text/image prompts。[官方 Chat API](https://docs.x.ai/developers/rest-api-reference/inference/chat-completions) `view_x_video` 是 X Search 返回视频的工具能力；不是客户端任意本地MP4的content支持。[官方工具计费说明](https://docs.x.ai/developers/pricing) 不给Grok通配 `video_url`。

## 其他已确认、但不是同一协议

Amazon官方用 `us.amazon.nova-lite-v1:0` 演示视频输入。Bedrock内容是 `{"video":{"format":"mp4","source":{"bytes":"<BASE64>"}}}`（InvokeModel JSON），或 `source.s3Location.uri`；Converse SDK 接收原始字节。既不是 `video_url` 也不是Google `inlineData`。官方样例代码有变量/注释混杂，实施应按API schema构建并测试，不逐字复制示例。此次未核完整大小限制，因此列作后续独立Bedrock适配。[官方 Nova 视频示例](https://docs.aws.amazon.com/nova/latest/userguide/modalities-video-examples.html)

百度千帆视觉文档搜到视频/Base64条目，但本轮正文读取失败；不据搜索摘要添加模型或序列化逻辑。其他自部署Qwen/GLM、vLLM和LM Studio端点须按实际server能力核对，不能据开源模型卡替用户端点自动开启。

## 实施与验证建议（研究推论）

1. 能力配置至少区分 `remoteUrl`、`dataUrl`、`fileUpload` 与协议；本地上传按钮需要 dataUrl 或已实现文件上传，而不是单一 video=true。
2. 先处理Qwen兼容Chat与MiniMax-M3 Chat这样官方完整确认路径。GLM直连本地输入在Base64证据未齐时保留未支持状态，不静默把视频丢弃或降为文本。
3. 精确模型ID映射同时保留用户手动配置；导入自定义前缀不应把未知供应商路由冒充直连。
4. 合同测试须检查 Qwen fps 在外、方舟 fps 在内、Responses 类型独立，以及不支持协议明确报错；大小检查计算Base64后的有效负载与整段会话请求体。
5. 实际上线应使用短MP4测试“文件内容真实被识别”，同时覆盖多轮、流式、工具调用与错误提示。此次仅完成文档审计，不声称已实网通过。


## 当前实现审计

以下是本地代码核查，不等于供应商调用实测。

| 位置 | 当前行为 | 结论与接入前要求 |
| --- | --- | --- |
| `src-tauri/src/chat/model/types.rs`，Video 转 Chat | 生成 `video_url.url=data:...;base64,...` | 对已核查的 Kimi Chat 契约正确；不是通用 OpenAI 标准 |
| `src-tauri/src/chat/model/gemini.rs`，Video 转 Part | `inlineData:{mimeType,data}` | 结构正确；不要加入 data URL 前缀 |
| `src-tauri/src/chat/video.rs::validate_model` | 只检查 API Key、Chat/Gemini 协议和能力布尔值 | 缺少供应商与传输格式判定；Gemini 模型经未核查的 Chat 入口也可能放行 |
| MOV MIME | 存储及 Chat 保留 `video/mov` | 本轮已在 Gemini 发送端改为 `video/quicktime`，兼容旧附件；不全局改写其他供应商格式 |
| `video.rs::MAX_VIDEO_REQUEST_BYTES` | 原为 `20 * 1024 * 1024` | 本轮已收紧为 20,000,000 字节；计入 JSON 引号等全部开销，并补充边界测试 |
| `video.rs::MAX_VIDEO_BYTES` | 上下文合计 14 MiB 原始视频 | 是 Kivio 自身限制，不能用来证明其他供应商的 Base64 上限也满足 |
| `modelMatching.ts` / `model_metadata.rs` | 精确匹配后仍可前缀、包含匹配 | 对私有别名、模型变体可能过度推断；视频能力应采用受约束的精确 ID/已验证别名 |
| `commands.rs` 获取模型 | 解析 `supports_video_in` | 只覆盖 Kimi 风格字段；其他厂商的模态字段尚未统一读取 |
| Video 的载体模型 | 只有内联字节和本地持久化路径 | 暂无远程视频、供应商 Files 引用、有效期、等待处理、取消上传和清理流程 |
| 请求测试 | 本轮补充 Gemini 流式/非流式 Blob、Chat data URL 与大小边界测试 | 锁定目前两种输出，仍不等于真实视频 E2E，更不覆盖新增供应商协议 |

## 后续实现约束

1. 将“模型可理解视频”和“当前路由可以发送本地视频”分开。解析结果至少应包含供应商、API 格式、内联/URL/文件引用载体、允许 MIME 和各自大小口径。
2. 默认启用只来自官方精确 ID、明确别名或供应商能力声明。未知项保持待确认；手动打开能力不能自动发明新序列化协议。
3. 按供应商定义传输策略后再开启其模型默认值。模型库不能先全量打勾、让发送阶段碰运气。
4. 对每条实际支持路径测试最终 HTTP JSON：角色、content 数组、裸 Base64/data URL 的区别、MIME、流式与非流式、完整请求上限、历史重发与取消。
5. Files 接入必须包括上传、处理状态、失败与取消、按供应商/账户隔离引用、过期恢复、删除或复用策略；不能只保存一个 file ID。
6. 文档核查与实际验证分开记账。发布前用各供应商的小型真实视频验证问答、续问、工具调用和错误反馈；没有实测的路由不标注“已完整接入”。

## 本次变更边界

本次交付以研究和格式审计为主，不批量启用新供应商的视频默认值，不声称新增接口已实现。上一轮无明确官方依据的裸 `kimi-k2.7` 视频标记已撤回；保留官方明确支持的 Code / HighSpeed 条目。已修正现有 Gemini MOV MIME 与十进制请求上限，并补测试。供应商/路由判定、Files、URL和新厂商协议仍列为后续接入前的必要工作，不能用能力识别测试通过来替代。

### 本地验证记录

- 修复前，Gemini 旧 MOV 的请求断言、20 MB 请求边界断言均失败；修复后通过。最初考虑统一替换 MOV MIME，但 OpenRouter 的官方格式表使用 `video/mov`，因此最终只在 Gemini 适配器转换，另测 Chat 保持原有 data URL。
- `npx vitest run src/data/modelCatalog.test.ts src/data/modelMatching.test.ts`：53 项通过；新增测试 ESLint 与 `git diff --check` 通过。
- Windows 测试脚本完成编译与 Common Controls manifest 配置后，因另一构建占锁，直接运行同一测试二进制。`video_` 4项、Gemini 26项、消息类型13项全部通过，去重41项。未重跑全仓库 Rust 测试。
- 没有向供应商发送真实视频、没有调用付费 API，没有将文档已支持的其他模型批量标记为“接入完成”。
