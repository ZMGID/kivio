use crate::chat::model_metadata::reasoning_efforts_for_model;

/// 由「每对话思考等级」解析出实际下发给模型的 `(thinking_enabled, thinking_level)`。
/// chat 不再跟随全局思考开关（全局开关只服务 lens / 快速翻译），未显式选档时落到默认档「high」。
/// - `"off"` → 强制关思考，不带等级。
/// - `"low"|"medium"|"high"|"xhigh"|"max"` → 开思考并带等级（适配器按家族映射为
///   reasoning_effort / output_config.effort）。等级是否被某模型接受由前端按模型 id 门控；
///   `xhigh` 仅 OpenAI GPT-5/Anthropic，`max` 仅 Anthropic。
/// - `None` 或其它未知值 → 默认档「high」（与前端 `ThinkingLevelSelector` 的 DEFAULT_LEVEL 一致）。
pub(crate) fn resolve_thinking(
    conv_level: Option<&str>,
    _global_enabled: bool,
) -> (bool, Option<String>) {
    match conv_level {
        Some("off") => (false, None),
        Some(level @ ("low" | "medium" | "high" | "xhigh" | "max")) => {
            (true, Some(level.to_string()))
        }
        _ => (true, Some("high".to_string())),
    }
}

/// 返回某模型支持的思考等级列表（数据来自模型库 `reasoningEfforts`）。供前端等级选择器决定显示哪些档。
#[tauri::command]
pub(crate) fn chat_reasoning_efforts_for_model(
    model: String,
    api_format: Option<String>,
) -> Vec<String> {
    reasoning_efforts_for_model(&model, api_format.as_deref().unwrap_or(""))
}
