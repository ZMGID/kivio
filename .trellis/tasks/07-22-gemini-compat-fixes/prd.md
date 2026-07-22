# Gemini 兼容性两连修：Vertex anyOf 校验 + 旧路径协议分叉

## 背景

用户反馈两个独立但同属 "Gemini 类供应商兼容性" 的 bug：

1. **Vertex Gemini 工具 schema 校验拒绝**：`present_artifacts` 的 `parameters` 顶层带约束型 `anyOf`（分支只有 `required`、无 `type`），`normalize_gemini_schema` 只折 nullable anyOf，约束型 anyOf 原样透传 → Vertex 提交前整请求被拒（`schema didn't specify the schema type field`）。官方 generativelanguage API 校验松未暴露。
2. **截图翻译/Lens/翻译器 404**：`api.rs` 旧调用路径（5 处 `format!("{}/chat/completions", base_url)`）硬编码 OpenAI 格式 + Bearer 鉴权，完全不读 provider 的 `api_format`；`api_format=gemini`（以及 anthropic_messages / openai_responses）的供应商被选为翻译/截图/Lens 模型时全挂（`path not found: /gemini/v1beta/chat/completions`）。聊天正常是因为走 `chat/model/` 多协议适配器。

## 任务地图

| 子任务 | 范围 | 规模 |
|---|---|---|
| `07-22-gemini-anyof-strip` | `chat/model/gemini.rs::normalize_gemini_schema` 剥约束型 anyOf/oneOf/allOf + 单测 | 小，先修 |
| `07-22-legacy-api-multiprotocol` | `api.rs` 翻译/OCR/Lens 调用迁到 `chat/model/` LanguageModelProvider 适配器 | 大，后做 |

两个子任务无实现依赖，可独立验收归档；顺序上先修小的。

## 跨子任务验收

- [ ] Vertex Gemini 供应商：聊天中带 `present_artifacts` 工具能正常发出请求（子任务 1）。
- [ ] `api_format=gemini` 供应商被选为截图翻译/Lens/翻译器模型时，请求走 Gemini 原生协议不再 404（子任务 2）。
- [ ] `cargo test --manifest-path src-tauri/Cargo.toml` 与前端 lint/typecheck 全绿。

## 完成定义

两个子任务均归档后，本父任务做最终集成回顾并归档。父任务本身无直接实现工作。
