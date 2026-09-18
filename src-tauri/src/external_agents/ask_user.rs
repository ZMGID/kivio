//! 外部 CLI 「问用户」接到 Kivio 已有卡片上的适配层。
//!
//! 宿主只认这一套：按 agent + 工具名找到 codec → 映射入参 → 弹同一张卡 →
//! 把答案编回这条 CLI 的线形状。**不改各 CLI 的线协议**（claude 仍走
//! `can_use_tool` + `updatedInput`，dsh 仍走 `session/ask`）。
//!
//! 新接一条 CLI：
//! 1. 下面 `CODECS` 加一行（工具名、parse / encode、形状不认识时怎么办）；
//! 2. 前端 `src/chat/askUserTools.ts` 把工具名加进识别表（否则会渲染成普通工具卡）。
//!
//! ponytail: 分发靠 `agent_id` + 大小写不敏感的**原名**，不折叠下划线。
//! `AskUserQuestion` 和 `ask_user_question` 折叠后一样，按折叠名会把 claude / dsh
//! 的答案编错。两个 CLI 真用了同一个工具名时，靠 agent_id 分开即可。

use serde_json::Value;

use crate::chat::ask_user::{
    AskUserOption, AskUserPromptPayload, AskUserQuestion, AskUserResponseResult,
};

/// 入参形状不认识时，宿主怎么收场。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnknownAskShape {
    /// 退回普通审批卡（claude：至少别让 `can_use_tool` 挂死）。
    FallbackApproval,
    /// 直接拒（dsh：`session/ask` 等的是 `answers`，不是 allow/deny）。
    Reject,
}

/// 一条外部 CLI 的问用户适配。
pub struct AskUserCodec {
    pub agent_id: &'static str,
    pub tools: &'static [&'static str],
    pub parse: fn(&Value) -> Option<AskUserPromptPayload>,
    pub encode: fn(&Value, &AskUserPromptPayload, &AskUserResponseResult) -> Value,
    pub unknown_shape: UnknownAskShape,
    /// 通道只为问用户开着时，普通工具就地放行，避免每个 Bash 都弹审批卡。
    pub auto_allow_ordinary_tools: bool,
    /// 没有 `--permission-prompt-tool` 也要建宿主（dsh 的 `session/ask`）。
    /// claude 仍只看 argv 上的 flag，避免空 argv 也挂上一条用不到的通道。
    pub opens_host: bool,
}

const CODECS: &[AskUserCodec] = &[
    AskUserCodec {
        agent_id: "claude",
        tools: &["AskUserQuestion"],
        parse: parse_claude,
        encode: encode_claude,
        unknown_shape: UnknownAskShape::FallbackApproval,
        auto_allow_ordinary_tools: false,
        opens_host: false,
    },
    AskUserCodec {
        agent_id: "dsh",
        tools: &["ask_user_question", "exit_plan_mode"],
        parse: parse_dsh,
        encode: encode_dsh,
        unknown_shape: UnknownAskShape::Reject,
        auto_allow_ordinary_tools: true,
        opens_host: true,
    },
    AskUserCodec {
        agent_id: "pi",
        tools: &[
            "PiExtensionConfirm",
            "PiExtensionSelect",
            "PiExtensionInput",
            "PiExtensionEditor",
        ],
        parse: parse_pi_extension_ui,
        encode: encode_pi_extension_ui,
        unknown_shape: UnknownAskShape::Reject,
        auto_allow_ordinary_tools: true,
        opens_host: true,
    },
    AskUserCodec {
        agent_id: "codex",
        tools: &["requestUserInput"],
        parse: parse_codex,
        encode: encode_codex,
        unknown_shape: UnknownAskShape::Reject,
        // Command / file / permissions approvals must reach the host card. requestUserInput
        // still goes through this codec; ordinary tools no longer auto-allow.
        auto_allow_ordinary_tools: false,
        opens_host: true,
    },
    AskUserCodec {
        agent_id: "codex",
        tools: &[
            "mcpServer/elicitation/request",
            "openai/elicitation",
            "openai/elicitation/create",
        ],
        parse: parse_mcp_elicitation,
        encode: encode_mcp_elicitation,
        unknown_shape: UnknownAskShape::Reject,
        auto_allow_ordinary_tools: false,
        opens_host: true,
    },
    AskUserCodec {
        agent_id: "cursor-agent",
        tools: &["cursor/ask_question"],
        parse: parse_cursor,
        encode: encode_cursor,
        unknown_shape: UnknownAskShape::Reject,
        auto_allow_ordinary_tools: true,
        opens_host: true,
    },
    AskUserCodec {
        agent_id: "grok",
        tools: &["elicitation/create"],
        parse: parse_mcp_elicitation,
        encode: encode_mcp_elicitation,
        unknown_shape: UnknownAskShape::Reject,
        auto_allow_ordinary_tools: false,
        opens_host: true,
    },
];

pub fn codec_for(agent_id: &str, tool_name: &str) -> Option<&'static AskUserCodec> {
    CODECS.iter().find(|codec| {
        codec.agent_id == agent_id
            && codec
                .tools
                .iter()
                .any(|name| name.eq_ignore_ascii_case(tool_name))
    })
}

pub fn matches_tool(agent_id: &str, tool_name: &str) -> bool {
    codec_for(agent_id, tool_name).is_some()
}

/// 这条外部 CLI 没有 `--permission-prompt-tool` 也要建宿主。
pub fn needs_host(agent_id: &str) -> bool {
    CODECS
        .iter()
        .any(|codec| codec.agent_id == agent_id && codec.opens_host)
}

pub fn auto_allow_ordinary_tools(agent_id: &str) -> bool {
    CODECS
        .iter()
        .any(|codec| codec.agent_id == agent_id && codec.auto_allow_ordinary_tools)
}

fn json_str<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value
        .get(key)?
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn json_bool(value: &Value, keys: &[&str]) -> bool {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_bool))
        .unwrap_or(false)
}

/// `options[].label` / `description` → 下标当选项 id。claude / dsh / 多数同类 CLI 都是这套。
fn parse_label_options(question: &Value) -> Vec<AskUserOption> {
    question
        .get("options")
        .and_then(|v| v.as_array())
        .map(|options| {
            options
                .iter()
                .enumerate()
                .filter_map(|(oi, option)| {
                    Some(AskUserOption {
                        id: oi.to_string(),
                        label: json_str(option, "label")?.to_string(),
                        description: json_str(option, "description").map(str::to_string),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn elicitation_schema(input: &Value) -> Option<&Value> {
    input
        .get("requestedSchema")
        .or_else(|| input.get("requested_schema"))
}

/// MCP form elicitation (`elicitation/create`) → Kivio's existing structured question card.
/// Keep the full property id as the answer key so the accepted `content` object can be rebuilt
/// without guessing. URL elicitation and oversized forms are declined by returning `None`.
fn parse_mcp_elicitation(input: &Value) -> Option<AskUserPromptPayload> {
    if input.get("url").is_some() || input.get("mode").is_some_and(|mode| mode != "form") {
        return None;
    }
    let schema = elicitation_schema(input)?;
    let schema_object = schema.as_object()?;
    if schema["type"] != "object"
        || schema_object.keys().any(|key| {
            !matches!(
                key.as_str(),
                "type"
                    | "properties"
                    | "required"
                    | "additionalProperties"
                    | "title"
                    | "description"
                    | "$schema"
            )
        })
        || schema
            .get("additionalProperties")
            .is_some_and(|value| value != &Value::Bool(false))
    {
        return None;
    }
    let properties = schema.get("properties")?.as_object()?;
    if properties.is_empty() || properties.len() > 4 {
        return None;
    }
    let required = match schema.get("required") {
        Some(value) => value.as_array()?.clone(),
        None => vec![],
    };
    if required
        .iter()
        .any(|value| value.as_str().is_none_or(|id| !properties.contains_key(id)))
    {
        return None;
    }
    let is_required = |id: &str| required.iter().any(|value| value.as_str() == Some(id));
    let mut questions = Vec::with_capacity(properties.len());
    for (id, property) in properties {
        if !crate::chat::ask_user::supports_value_schema(property) {
            return None;
        }
        if id.is_empty() || id.len() > 40 {
            return None;
        }
        let title = json_str(property, "title").unwrap_or(id);
        let description = json_str(property, "description");
        let mut prompt = match description {
            Some(description) if description != title => format!("{title}\n\n{description}"),
            _ => title.to_string(),
        };
        if is_required(id) {
            prompt.push_str("\n\n必填");
        }

        let options = if let Some(values) = property.get("enum").and_then(Value::as_array) {
            if !(2..=6).contains(&values.len()) {
                return None;
            }
            values
                .iter()
                .enumerate()
                .map(|(index, value)| AskUserOption {
                    id: index.to_string(),
                    label: value
                        .as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| value.to_string()),
                    description: None,
                })
                .collect()
        } else if property.get("type").and_then(Value::as_str) == Some("boolean") {
            vec![
                AskUserOption {
                    id: "true".to_string(),
                    label: "是".to_string(),
                    description: None,
                },
                AskUserOption {
                    id: "false".to_string(),
                    label: "否".to_string(),
                    description: None,
                },
            ]
        } else {
            Vec::new()
        };
        questions.push(AskUserQuestion {
            required: is_required(id),
            value_schema: Some(property.clone()),
            id: id.clone(),
            prompt,
            allow_multiple: false,
            allow_custom: options.is_empty(),
            options,
        });
    }
    Some(AskUserPromptPayload {
        title: json_str(input, "message").map(str::to_string),
        questions,
    })
}

fn encode_mcp_elicitation(
    _original_input: &Value,
    prompt: &AskUserPromptPayload,
    answered: &AskUserResponseResult,
) -> Value {
    use crate::chat::ask_user::{
        validate_response, ASK_USER_PHASE_ANSWERED, ASK_USER_PHASE_SKIPPED,
    };
    if answered.phase != ASK_USER_PHASE_ANSWERED {
        return serde_json::json!({"action": if answered.phase == ASK_USER_PHASE_SKIPPED {"decline"} else {"cancel"}});
    }
    let Ok(answered) = validate_response(prompt, answered.clone()) else {
        return serde_json::json!({"action":"decline"});
    };
    let mut content = serde_json::Map::new();
    for question in &prompt.questions {
        let Some(answer) = answered.answers.get(&question.id) else {
            continue;
        };
        let Ok(value) = crate::chat::ask_user::answer_value(question, answer) else {
            return serde_json::json!({"action":"decline"});
        };
        content.insert(question.id.clone(), value);
    }
    serde_json::json!({ "action": "accept", "content": content })
}

fn parse_pi_extension_ui(input: &Value) -> Option<AskUserPromptPayload> {
    let method = json_str(input, "method")?;
    let title = json_str(input, "title").map(str::to_string);
    let (prompt, options, allow_custom) = match method {
        "confirm" => (
            json_str(input, "message")
                .or_else(|| title.as_deref())?
                .to_string(),
            vec![
                AskUserOption {
                    id: "confirm".to_string(),
                    label: "确认".to_string(),
                    description: None,
                },
                AskUserOption {
                    id: "cancel".to_string(),
                    label: "取消".to_string(),
                    description: None,
                },
            ],
            false,
        ),
        "select" => {
            let options = input
                .get("options")?
                .as_array()?
                .iter()
                .enumerate()
                .filter_map(|(index, value)| {
                    value.as_str().map(|label| AskUserOption {
                        id: index.to_string(),
                        label: label.to_string(),
                        description: None,
                    })
                })
                .collect::<Vec<_>>();
            if options.is_empty() {
                return None;
            }
            (
                title
                    .clone()
                    .unwrap_or_else(|| "请选择一个选项".to_string()),
                options,
                false,
            )
        }
        "input" => (
            json_str(input, "placeholder")
                .or_else(|| title.as_deref())
                .unwrap_or("请输入内容")
                .to_string(),
            Vec::new(),
            true,
        ),
        "editor" => (
            title.clone().unwrap_or_else(|| "请输入内容".to_string()),
            Vec::new(),
            true,
        ),
        _ => return None,
    };
    Some(AskUserPromptPayload {
        title,
        questions: vec![AskUserQuestion {
            required: true,
            value_schema: None,
            id: "0".to_string(),
            prompt,
            options,
            allow_multiple: false,
            allow_custom,
        }],
    })
}

fn encode_pi_extension_ui(
    original_input: &Value,
    _prompt: &AskUserPromptPayload,
    answered: &AskUserResponseResult,
) -> Value {
    let Some(answer) = answered.answers.get("0") else {
        return serde_json::json!({ "cancelled": true });
    };
    match original_input.get("method").and_then(Value::as_str) {
        Some("confirm") => serde_json::json!({
            "confirmed": answer.selected_option_ids.iter().any(|id| id == "confirm")
        }),
        Some("select") => {
            let value = answer
                .selected_option_ids
                .first()
                .and_then(|id| id.parse::<usize>().ok())
                .and_then(|index| original_input.get("options")?.as_array()?.get(index))
                .and_then(Value::as_str);
            value
                .map(|value| serde_json::json!({ "value": value }))
                .unwrap_or_else(|| serde_json::json!({ "cancelled": true }))
        }
        Some("input" | "editor") => answer
            .custom_text
            .as_deref()
            .map(|value| serde_json::json!({ "value": value }))
            .unwrap_or_else(|| serde_json::json!({ "cancelled": true })),
        _ => serde_json::json!({ "cancelled": true }),
    }
}

/// claude `AskUserQuestion` 的入参 → Kivio 问用户卡片。
///
/// 官方形状：`{"questions":[{"question":"…","header":"…","multiSelect":bool,
///   "options":[{"label":"…","description":"…"}]}]}`
///
/// 选项 id 用**下标**（claude 的选项没有 id，只有 label），答复时再按同一个下标翻回
/// label —— 见 `encode_claude`。形状不认识返回 `None`，调用方退回普通审批卡。
fn parse_claude(input: &Value) -> Option<AskUserPromptPayload> {
    let raw = input.get("questions")?.as_array()?;
    let questions: Vec<AskUserQuestion> = raw
        .iter()
        .enumerate()
        .filter_map(|(qi, question)| {
            let text = json_str(question, "question")?.to_string();
            let options = parse_label_options(question);
            if options.is_empty() {
                return None;
            }
            Some(AskUserQuestion {
                id: qi.to_string(),
                required: true,
                value_schema: None,
                prompt: text,
                options,
                allow_multiple: json_bool(question, &["multiSelect"]),
                // claude 的 schema 里没有「自定义文本」这一档，但用户总该能不选任何预设项
                // 直接说自己的想法 —— 这是 Kivio 卡片本来就有的能力，白给。
                allow_custom: true,
            })
        })
        .collect();
    (!questions.is_empty()).then_some(AskUserPromptPayload {
        title: None,
        questions,
    })
}

/// 用户的选择 → claude 要的 `updatedInput`。
///
/// 官方形状：`{"questions": <原样回传>, "answers": {"<问题文本>": "<选中的 label>"}}`。
/// 多选时把多个 label 用 `, ` 拼起来（官方文档只给了单选的例子，多选的分隔符**未核实**；
/// 拼串至少保证 claude 拿到的是它认识的字符串类型，而不是一个它可能不接受的数组）。
/// 用户填的自定义文本优先于预设项 —— 那是他真正想说的话。
///
/// **回的是「原入参 + answers」而不是重新拼一个 `{questions, answers}`**（paseo 的写法：
/// `{...permission.request.input, answers}`）。`updatedInput` 会**整个替换**这次调用的入参，
/// 自己拼就意味着 CLI 哪天给 schema 加个字段、我们就静默把它丢了 —— paseo 那边这个坑是
/// 真踩过的（CHANGELOG #760「Answering an interactive question from a Claude agent now
/// reaches Claude correctly instead of being dropped」）。
fn encode_claude(
    original_input: &Value,
    _prompt: &AskUserPromptPayload,
    answered: &AskUserResponseResult,
) -> Value {
    let mut answers = serde_json::Map::new();
    let raw = original_input
        .get("questions")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    for (qi, question) in raw.iter().enumerate() {
        let Some(text) = question.get("question").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(answer) = answered.answers.get(&qi.to_string()) else {
            continue;
        };
        if let Some(custom) = answer
            .custom_text
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            answers.insert(text.to_string(), Value::String(custom.to_string()));
            continue;
        }
        let labels: Vec<String> = answer
            .selected_option_ids
            .iter()
            .filter_map(|id| {
                let index: usize = id.parse().ok()?;
                question
                    .get("options")?
                    .as_array()?
                    .get(index)?
                    .get("label")?
                    .as_str()
                    .map(str::to_string)
            })
            .collect();
        if !labels.is_empty() {
            answers.insert(text.to_string(), Value::String(labels.join(", ")));
        }
    }
    let mut updated = original_input
        .as_object()
        .cloned()
        .unwrap_or_else(serde_json::Map::new);
    updated.insert("questions".to_string(), Value::Array(raw));
    updated.insert("answers".to_string(), Value::Object(answers));
    Value::Object(updated)
}

/// dsh `session/ask` 的官方问题形状 → Kivio 问用户卡片。
///
/// 入参（`dsh-user-questions` / `dsh-tool-ask-user`）：
/// `{ questions: [{ id, question, header?, detail?, options?: [{ label, description? }],
///   multiSelect? | multi_select? }] }`
///
/// 选项 id 用下标，答复时再翻回 **label**（官方 `selected` 装的是 label，不是 id）。
/// 没有选项的问题仍可成卡（`allow_custom`），对应官方「纯文本作答」。
fn parse_dsh(input: &Value) -> Option<AskUserPromptPayload> {
    let raw = input.get("questions")?.as_array()?;
    let mut title = None;
    let questions: Vec<AskUserQuestion> = raw
        .iter()
        .filter_map(|question| {
            let id = json_str(question, "id")?.to_string();
            let text = json_str(question, "question")?.to_string();
            let prompt = match json_str(question, "detail") {
                Some(detail) => format!("{text}\n\n{detail}"),
                None => text,
            };
            if title.is_none() {
                title = json_str(question, "header").map(str::to_string);
            }
            Some(AskUserQuestion {
                id,
                prompt,
                options: parse_label_options(question),
                required: true,
                value_schema: None,
                allow_multiple: json_bool(question, &["multiSelect", "multi_select"]),
                allow_custom: true,
            })
        })
        .collect();
    (!questions.is_empty()).then_some(AskUserPromptPayload { title, questions })
}

/// 用户的选择 → dsh 官方 `AskUserQuestionAnswer`。
///
/// 单选时 `custom` 覆盖选项，`selected` 为空；多选时 `custom` 可以和 label 并存。
/// 跳过的题保留 `id` + 空 `selected`（官方允许用它在一批里占位）。
fn encode_dsh(
    _original_input: &Value,
    prompt: &AskUserPromptPayload,
    answered: &AskUserResponseResult,
) -> Value {
    let answers: Vec<Value> = prompt
        .questions
        .iter()
        .map(|question| {
            let Some(answer) = answered.answers.get(&question.id) else {
                return serde_json::json!({
                    "id": question.id,
                    "selected": [],
                });
            };
            let custom = answer
                .custom_text
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let selected: Vec<String> = if custom.is_some() && !question.allow_multiple {
                Vec::new()
            } else {
                answer
                    .selected_option_ids
                    .iter()
                    .filter_map(|id| {
                        question
                            .options
                            .iter()
                            .find(|option| option.id == *id)
                            .map(|option| option.label.clone())
                    })
                    .collect()
            };
            match custom {
                Some(custom) => serde_json::json!({
                    "id": question.id,
                    "selected": selected,
                    "custom": custom,
                }),
                None => serde_json::json!({
                    "id": question.id,
                    "selected": selected,
                }),
            }
        })
        .collect();
    serde_json::json!({ "answers": answers })
}

/// Codex `item/tool/requestUserInput` 的入参 → Kivio 问用户卡片。
///
/// 官方形状（app-server v2）：`{ questions: [{ id, header, question, isOther?,
///   isSecret?, options?: [{ label, description }] }] }`。
/// 选项 id 用下标，答复时再翻回 **label**（响应是 `{ answers: { <id>: { answers: [label…] } } }`）。
/// `isOther` 或没有预设项时开放自定义文本。
fn parse_codex(input: &Value) -> Option<AskUserPromptPayload> {
    let raw = input.get("questions")?.as_array()?;
    let mut title = None;
    let questions: Vec<AskUserQuestion> = raw
        .iter()
        .filter_map(|question| {
            let id = json_str(question, "id")?.to_string();
            let text = json_str(question, "question")?.to_string();
            if title.is_none() {
                title = json_str(question, "header").map(str::to_string);
            }
            let options = parse_label_options(question);
            Some(AskUserQuestion {
                id,
                prompt: text,
                required: true,
                value_schema: None,
                options,
                allow_multiple: json_bool(question, &["multiSelect", "multi_select"]),
                allow_custom: json_bool(question, &["isOther", "is_other"])
                    || question
                        .get("options")
                        .and_then(Value::as_array)
                        .map(|options| options.is_empty())
                        .unwrap_or(true),
            })
        })
        .collect();
    (!questions.is_empty()).then_some(AskUserPromptPayload { title, questions })
}

/// 用户的选择 → Codex `ToolRequestUserInputResponse`。
///
/// `{ answers: { "<questionId>": { "answers": ["<label or custom>", …] } } }`。
fn encode_codex(
    _original_input: &Value,
    prompt: &AskUserPromptPayload,
    answered: &AskUserResponseResult,
) -> Value {
    let mut answers = serde_json::Map::new();
    for question in &prompt.questions {
        let Some(answer) = answered.answers.get(&question.id) else {
            answers.insert(question.id.clone(), serde_json::json!({ "answers": [] }));
            continue;
        };
        let custom = answer
            .custom_text
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let mut labels: Vec<String> = answer
            .selected_option_ids
            .iter()
            .filter_map(|id| {
                question
                    .options
                    .iter()
                    .find(|option| option.id == *id)
                    .map(|option| option.label.clone())
            })
            .collect();
        if let Some(custom) = custom {
            if !question.allow_multiple {
                labels.clear();
            }
            labels.push(custom.to_string());
        }
        answers.insert(
            question.id.clone(),
            serde_json::json!({ "answers": labels }),
        );
    }
    serde_json::json!({ "answers": answers })
}

/// Cursor ACP `cursor/ask_question` → Kivio 问用户卡片。
///
/// 官方形状：`{ toolCallId, title?, questions: [{ id, prompt, options: [{id,label}],
///   allowMultiple? }] }`。选项 id 原样回传（不是下标）。
fn parse_cursor(input: &Value) -> Option<AskUserPromptPayload> {
    let raw = input.get("questions")?.as_array()?;
    let title = json_str(input, "title").map(str::to_string);
    let questions: Vec<AskUserQuestion> = raw
        .iter()
        .filter_map(|question| {
            let id = json_str(question, "id")?.to_string();
            let prompt = json_str(question, "prompt")?.to_string();
            let options = question
                .get("options")
                .and_then(Value::as_array)
                .map(|options| {
                    options
                        .iter()
                        .filter_map(|option| {
                            Some(AskUserOption {
                                id: json_str(option, "id")?.to_string(),
                                label: json_str(option, "label")?.to_string(),
                                description: json_str(option, "description").map(str::to_string),
                            })
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if options.is_empty() {
                return None;
            }
            Some(AskUserQuestion {
                id,
                prompt,
                options,
                required: true,
                value_schema: None,
                allow_multiple: json_bool(question, &["allowMultiple", "allow_multiple"]),
                allow_custom: false,
            })
        })
        .collect();
    (!questions.is_empty()).then_some(AskUserPromptPayload { title, questions })
}

/// 用户的选择 → Cursor `CursorAskQuestionResponse`。
///
/// `{ outcome: { outcome: "answered", answers: [{ questionId, selectedOptionIds }] } }`。
fn encode_cursor(
    _original_input: &Value,
    prompt: &AskUserPromptPayload,
    answered: &AskUserResponseResult,
) -> Value {
    let answers: Vec<Value> = prompt
        .questions
        .iter()
        .map(|question| {
            let selected = answered
                .answers
                .get(&question.id)
                .map(|answer| answer.selected_option_ids.clone())
                .unwrap_or_default();
            serde_json::json!({
                "questionId": question.id,
                "selectedOptionIds": selected,
            })
        })
        .collect();
    serde_json::json!({
        "outcome": {
            "outcome": "answered",
            "answers": answers,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::ask_user::{AskUserAnswer, ASK_USER_PHASE_ANSWERED};
    use std::collections::HashMap;

    #[test]
    fn codec_dispatch_is_by_agent_and_exact_tool_name() {
        assert!(codec_for("claude", "AskUserQuestion").is_some());
        assert!(codec_for("claude", "askuserquestion").is_some());
        assert!(codec_for("claude", "ask_user_question").is_none());
        assert!(codec_for("claude", "ExitPlanMode").is_none());
        assert!(codec_for("dsh", "ask_user_question").is_some());
        assert!(codec_for("dsh", "exit_plan_mode").is_some());
        assert!(codec_for("dsh", "AskUserQuestion").is_none());
        assert!(codec_for("dsh", "bash").is_none());
        assert!(codec_for("cursor", "AskUserQuestion").is_none());
        assert!(codec_for("cursor-agent", "cursor/ask_question").is_some());
        assert!(codec_for("cursor-agent", "AskUserQuestion").is_none());
        assert!(codec_for("codex", "requestUserInput").is_some());
        assert!(codec_for("codex", "AskUserQuestion").is_none());
        assert!(needs_host("dsh"));
        assert!(!needs_host("claude"));
        assert!(!needs_host("cursor"));
        assert!(needs_host("cursor-agent"));
        assert!(needs_host("pi"));
        assert!(needs_host("codex"));
        assert!(auto_allow_ordinary_tools("cursor-agent"));
        assert!(auto_allow_ordinary_tools("dsh"));
        assert!(!auto_allow_ordinary_tools("codex"));
        assert!(!auto_allow_ordinary_tools("claude"));
    }

    #[test]
    fn unknown_shape_policy_differs_by_codec() {
        assert_eq!(
            codec_for("claude", "AskUserQuestion")
                .expect("claude codec")
                .unknown_shape,
            UnknownAskShape::FallbackApproval
        );
        assert_eq!(
            codec_for("dsh", "ask_user_question")
                .expect("dsh codec")
                .unknown_shape,
            UnknownAskShape::Reject
        );
    }

    #[test]
    fn pi_extension_ui_maps_confirm_select_and_input() {
        let confirm = serde_json::json!({
            "method": "confirm",
            "title": "Delete file?",
            "message": "This cannot be undone",
        });
        let prompt = parse_pi_extension_ui(&confirm).expect("confirm prompt");
        assert_eq!(prompt.questions[0].options.len(), 2);
        let answered = AskUserResponseResult {
            phase: ASK_USER_PHASE_ANSWERED.to_string(),
            answers: HashMap::from([(
                "0".to_string(),
                AskUserAnswer {
                    selected_option_ids: vec!["cancel".to_string()],
                    custom_text: None,
                },
            )]),
        };
        assert_eq!(
            encode_pi_extension_ui(&confirm, &prompt, &answered),
            serde_json::json!({ "confirmed": false })
        );

        let select = serde_json::json!({
            "method": "select",
            "title": "Runtime",
            "options": ["Node", "Bun"],
        });
        let prompt = parse_pi_extension_ui(&select).expect("select prompt");
        let answered = AskUserResponseResult {
            phase: ASK_USER_PHASE_ANSWERED.to_string(),
            answers: HashMap::from([(
                "0".to_string(),
                AskUserAnswer {
                    selected_option_ids: vec!["1".to_string()],
                    custom_text: None,
                },
            )]),
        };
        assert_eq!(
            encode_pi_extension_ui(&select, &prompt, &answered),
            serde_json::json!({ "value": "Bun" })
        );

        let input = serde_json::json!({ "method": "input", "title": "Branch" });
        let prompt = parse_pi_extension_ui(&input).expect("input prompt");
        assert!(prompt.questions[0].allow_custom);
        let answered = AskUserResponseResult {
            phase: ASK_USER_PHASE_ANSWERED.to_string(),
            answers: HashMap::from([(
                "0".to_string(),
                AskUserAnswer {
                    selected_option_ids: Vec::new(),
                    custom_text: Some("feature/pi-rpc".to_string()),
                },
            )]),
        };
        assert_eq!(
            encode_pi_extension_ui(&input, &prompt, &answered),
            serde_json::json!({ "value": "feature/pi-rpc" })
        );
    }

    /// claude 的 `AskUserQuestion` 入参必须能映射成 Kivio 的问用户卡片，否则这个功能
    /// 就退回到「当场拒」——claude 在 Kivio 里从此不能反问用户。
    #[test]
    fn claude_ask_user_input_maps_to_the_ask_user_card() {
        let input = serde_json::json!({
            "questions": [{
                "question": "用哪种缓存？",
                "header": "Cache",
                "multiSelect": false,
                "options": [
                    { "label": "Redis", "description": "跨进程共享" },
                    { "label": "内存", "description": null },
                ],
            }],
        });
        let prompt = parse_claude(&input).expect("必须能映射");
        assert_eq!(prompt.questions.len(), 1);
        let question = &prompt.questions[0];
        assert_eq!(question.prompt, "用哪种缓存？");
        assert!(!question.allow_multiple);
        assert_eq!(question.options[0].id, "0");
        assert_eq!(question.options[0].label, "Redis");
        assert_eq!(
            question.options[0].description.as_deref(),
            Some("跨进程共享")
        );
        assert_eq!(question.options[1].id, "1");
        assert!(question.options[1].description.is_none());
    }

    /// 形状不认识时必须返回 `None` —— 调用方据此退回普通审批卡，而不是静默吞掉询问
    /// （吞掉 = CLI 那条 promise 永远等不到回复 = 整轮挂死）。
    #[test]
    fn unknown_claude_ask_user_shapes_degrade_to_none() {
        assert!(parse_claude(&serde_json::json!({})).is_none());
        assert!(parse_claude(&serde_json::json!({
            "questions": [{ "question": "空的", "options": [] }],
        }))
        .is_none());
    }

    /// 答复必须按官方形状回：`{questions: <原样>, answers: {"<问题文本>": "<label>"}}`。
    /// 键是**问题文本**而不是下标 —— 用错就等于没答，claude 会当成没选。
    #[test]
    fn claude_answers_use_the_question_text_as_key() {
        let input = serde_json::json!({
            "questions": [{
                "question": "用哪种缓存？",
                "options": [{ "label": "Redis" }, { "label": "内存" }],
            }],
        });
        let answered = AskUserResponseResult {
            phase: ASK_USER_PHASE_ANSWERED.to_string(),
            answers: HashMap::from([(
                "0".to_string(),
                AskUserAnswer {
                    selected_option_ids: vec!["1".to_string()],
                    custom_text: None,
                },
            )]),
        };
        let prompt = parse_claude(&input).expect("必须能映射");
        let updated = encode_claude(&input, &prompt, &answered);
        assert_eq!(
            updated["answers"]["用哪种缓存？"],
            serde_json::json!("内存")
        );
        assert_eq!(updated["questions"], input["questions"]);
    }

    /// 用户填的自定义文本优先于预设项 —— 那是他真正想说的话。
    #[test]
    fn claude_custom_text_wins_over_selected_options() {
        let input = serde_json::json!({
            "questions": [{ "question": "怎么做？", "options": [{ "label": "A" }] }],
        });
        let answered = AskUserResponseResult {
            phase: ASK_USER_PHASE_ANSWERED.to_string(),
            answers: HashMap::from([(
                "0".to_string(),
                AskUserAnswer {
                    selected_option_ids: vec!["0".to_string()],
                    custom_text: Some("都不要，换个思路".to_string()),
                },
            )]),
        };
        let prompt = parse_claude(&input).expect("必须能映射");
        let updated = encode_claude(&input, &prompt, &answered);
        assert_eq!(
            updated["answers"]["怎么做？"],
            serde_json::json!("都不要，换个思路")
        );
    }

    /// `updatedInput` **整个替换**这次调用的入参，所以原入参里我们不认识的字段必须原样带回。
    #[test]
    fn claude_unknown_input_fields_survive_the_round_trip() {
        let input = serde_json::json!({
            "questions": [{ "question": "去哪？", "options": [{ "label": "左" }] }],
            "someFutureField": { "kept": true },
        });
        let answered = AskUserResponseResult {
            phase: ASK_USER_PHASE_ANSWERED.to_string(),
            answers: HashMap::from([(
                "0".to_string(),
                AskUserAnswer {
                    selected_option_ids: vec!["0".to_string()],
                    custom_text: None,
                },
            )]),
        };
        let prompt = parse_claude(&input).expect("必须能映射");
        let updated = encode_claude(&input, &prompt, &answered);
        assert_eq!(updated["someFutureField"], input["someFutureField"]);
        assert_eq!(updated["questions"], input["questions"]);
        assert_eq!(updated["answers"]["去哪？"], serde_json::json!("左"));
    }

    #[test]
    fn dsh_ask_user_input_maps_official_ids_and_free_text() {
        let input = serde_json::json!({
            "questions": [{
                "id": "runtime",
                "question": "用哪个运行时？",
                "header": "Runtime",
                "detail": "影响脚本和依赖。",
                "multiSelect": false,
                "options": [
                    { "label": "Bun", "description": "更快" },
                    { "label": "Node" },
                ],
            }, {
                "id": "note",
                "question": "还有补充吗？",
                "multi_select": false,
            }],
        });
        let prompt = parse_dsh(&input).expect("必须能映射");
        assert_eq!(prompt.title.as_deref(), Some("Runtime"));
        assert_eq!(prompt.questions.len(), 2);
        assert_eq!(prompt.questions[0].id, "runtime");
        assert_eq!(
            prompt.questions[0].prompt,
            "用哪个运行时？\n\n影响脚本和依赖。"
        );
        assert_eq!(prompt.questions[0].options[0].id, "0");
        assert_eq!(prompt.questions[0].options[0].label, "Bun");
        assert!(prompt.questions[0].allow_custom);
        assert_eq!(prompt.questions[1].id, "note");
        assert!(prompt.questions[1].options.is_empty());
        assert!(prompt.questions[1].allow_custom);
    }

    #[test]
    fn unknown_dsh_ask_user_shapes_degrade_to_none() {
        assert!(parse_dsh(&serde_json::json!({})).is_none());
        assert!(parse_dsh(&serde_json::json!({
            "questions": [{ "question": "没有 id" }],
        }))
        .is_none());
    }

    #[test]
    fn dsh_ask_user_answers_echo_labels_and_official_custom_rules() {
        let prompt = AskUserPromptPayload {
            title: None,
            questions: vec![
                AskUserQuestion {
                    id: "drink".to_string(),
                    required: true,
                    value_schema: None,
                    prompt: "喝什么？".to_string(),
                    options: vec![
                        AskUserOption {
                            id: "0".to_string(),
                            label: "茶".to_string(),
                            description: None,
                        },
                        AskUserOption {
                            id: "1".to_string(),
                            label: "咖啡".to_string(),
                            description: None,
                        },
                    ],
                    allow_multiple: false,
                    allow_custom: true,
                },
                AskUserQuestion {
                    id: "langs".to_string(),
                    required: true,
                    value_schema: None,
                    prompt: "会哪些？".to_string(),
                    options: vec![
                        AskUserOption {
                            id: "0".to_string(),
                            label: "Rust".to_string(),
                            description: None,
                        },
                        AskUserOption {
                            id: "1".to_string(),
                            label: "Go".to_string(),
                            description: None,
                        },
                    ],
                    allow_multiple: true,
                    allow_custom: true,
                },
            ],
        };
        let answered = AskUserResponseResult {
            phase: ASK_USER_PHASE_ANSWERED.to_string(),
            answers: HashMap::from([
                (
                    "drink".to_string(),
                    AskUserAnswer {
                        selected_option_ids: vec!["0".to_string()],
                        custom_text: Some("白开水".to_string()),
                    },
                ),
                (
                    "langs".to_string(),
                    AskUserAnswer {
                        selected_option_ids: vec!["0".to_string(), "1".to_string()],
                        custom_text: Some("也写 TS".to_string()),
                    },
                ),
            ]),
        };
        let payload = encode_dsh(&Value::Null, &prompt, &answered);
        assert_eq!(
            payload["answers"][0],
            serde_json::json!({ "id": "drink", "selected": [], "custom": "白开水" })
        );
        assert_eq!(
            payload["answers"][1],
            serde_json::json!({
                "id": "langs",
                "selected": ["Rust", "Go"],
                "custom": "也写 TS"
            })
        );
    }

    #[test]
    fn codex_request_user_input_maps_ids_options_and_free_text() {
        let input = serde_json::json!({
            "itemId": "item-1",
            "questions": [{
                "id": "runtime",
                "header": "Runtime",
                "question": "用哪个运行时？",
                "isOther": false,
                "options": [
                    { "label": "Bun", "description": "更快" },
                    { "label": "Node" },
                ],
            }, {
                "id": "note",
                "header": "Note",
                "question": "还有补充吗？",
                "isOther": true,
            }],
        });
        let prompt = parse_codex(&input).expect("必须能映射");
        assert_eq!(prompt.title.as_deref(), Some("Runtime"));
        assert_eq!(prompt.questions.len(), 2);
        assert_eq!(prompt.questions[0].id, "runtime");
        assert_eq!(prompt.questions[0].options[0].label, "Bun");
        assert!(!prompt.questions[0].allow_custom);
        assert_eq!(prompt.questions[1].id, "note");
        assert!(prompt.questions[1].allow_custom);
    }

    #[test]
    fn unknown_codex_ask_user_shapes_degrade_to_none() {
        assert!(parse_codex(&serde_json::json!({})).is_none());
        assert!(parse_codex(&serde_json::json!({
            "questions": [{ "question": "没有 id" }],
        }))
        .is_none());
    }

    #[test]
    fn codex_answers_use_question_ids_and_labels() {
        let input = serde_json::json!({
            "questions": [{
                "id": "drink",
                "question": "喝什么？",
                "options": [{ "label": "茶" }, { "label": "咖啡" }],
            }],
        });
        let prompt = parse_codex(&input).expect("必须能映射");
        let answered = AskUserResponseResult {
            phase: ASK_USER_PHASE_ANSWERED.to_string(),
            answers: HashMap::from([(
                "drink".to_string(),
                AskUserAnswer {
                    selected_option_ids: vec!["1".to_string()],
                    custom_text: None,
                },
            )]),
        };
        let payload = encode_codex(&input, &prompt, &answered);
        assert_eq!(
            payload["answers"]["drink"],
            serde_json::json!({ "answers": ["咖啡"] })
        );
    }

    #[test]
    fn codex_custom_text_replaces_single_select() {
        let input = serde_json::json!({
            "questions": [{
                "id": "drink",
                "question": "喝什么？",
                "isOther": true,
                "options": [{ "label": "茶" }],
            }],
        });
        let prompt = parse_codex(&input).expect("必须能映射");
        let answered = AskUserResponseResult {
            phase: ASK_USER_PHASE_ANSWERED.to_string(),
            answers: HashMap::from([(
                "drink".to_string(),
                AskUserAnswer {
                    selected_option_ids: vec!["0".to_string()],
                    custom_text: Some("白开水".to_string()),
                },
            )]),
        };
        let payload = encode_codex(&input, &prompt, &answered);
        assert_eq!(
            payload["answers"]["drink"],
            serde_json::json!({ "answers": ["白开水"] })
        );
    }

    #[test]
    fn cursor_ask_question_maps_option_ids() {
        let input = serde_json::json!({
            "toolCallId": "call_123",
            "title": "Need input",
            "questions": [{
                "id": "q1",
                "prompt": "Which mode should I use?",
                "options": [
                    { "id": "agent", "label": "Agent" },
                    { "id": "plan", "label": "Plan" }
                ],
                "allowMultiple": false
            }],
        });
        let prompt = parse_cursor(&input).expect("cursor ask");
        assert_eq!(prompt.title.as_deref(), Some("Need input"));
        assert_eq!(prompt.questions[0].id, "q1");
        assert_eq!(prompt.questions[0].options[0].id, "agent");
        assert!(!prompt.questions[0].allow_multiple);
        let answered = AskUserResponseResult {
            phase: ASK_USER_PHASE_ANSWERED.to_string(),
            answers: HashMap::from([(
                "q1".to_string(),
                AskUserAnswer {
                    selected_option_ids: vec!["plan".to_string()],
                    custom_text: None,
                },
            )]),
        };
        let payload = encode_cursor(&input, &prompt, &answered);
        assert_eq!(payload["outcome"]["outcome"], "answered");
        assert_eq!(
            payload["outcome"]["answers"],
            serde_json::json!([{
                "questionId": "q1",
                "selectedOptionIds": ["plan"]
            }])
        );
        assert!(parse_cursor(&serde_json::json!({})).is_none());
        assert!(parse_cursor(&serde_json::json!({
            "questions": [{ "id": "q1", "prompt": "无选项" }],
        }))
        .is_none());
    }

    #[test]
    fn mcp_numeric_answers_are_validated_before_encoding() {
        let codec = codec_for("grok", "elicitation/create").unwrap();
        let input = serde_json::json!({"requestedSchema": {"type":"object", "required":["count"],
            "properties":{"count":{"type":"integer", "minimum":0, "maximum":5}}}});
        let prompt = (codec.parse)(&input).unwrap();
        for text in [
            "not a number",
            "1.5",
            "-1",
            "6",
            "NaN",
            "1e999",
            "9007199254740993",
            "1.0000000000000001",
        ] {
            let response = AskUserResponseResult {
                phase: ASK_USER_PHASE_ANSWERED.into(),
                answers: HashMap::from([(
                    "count".into(),
                    AskUserAnswer {
                        selected_option_ids: vec![],
                        custom_text: Some(text.into()),
                    },
                )]),
            };
            let error =
                crate::chat::ask_user::validate_response(&prompt, response.clone()).unwrap_err();
            assert!(error.contains("count"), "{text}: {error}");
            assert_eq!(
                (codec.encode)(&input, &prompt, &response),
                serde_json::json!({"action":"decline"})
            );
        }
    }

    #[test]
    fn mcp_forms_reject_unsupported_assertions_and_keep_empty_optional_forms() {
        let codec = codec_for("grok", "elicitation/create").unwrap();
        for property in [
            serde_json::json!({"type":"object"}),
            serde_json::json!({"type":"null"}),
            serde_json::json!({"type":["string","null"]}),
            serde_json::json!({"type":"string","pattern":"^a"}),
            serde_json::json!({"type":"string","format":"email"}),
            serde_json::json!({"type":"number","multipleOf":2}),
            serde_json::json!({"type":"string","minLength":-1}),
            serde_json::json!({"type":"integer","enum":[1,"two"]}),
        ] {
            assert!(
                (codec.parse)(&serde_json::json!({"requestedSchema":{"type":"object",
                "properties":{"field":property}}}))
                .is_none(),
                "{property}"
            );
        }
        let input = serde_json::json!({"requestedSchema":{"type":"object", "properties":{
            "note":{"type":"string", "default":"do not insert"}}}});
        let prompt = (codec.parse)(&input).unwrap();
        let response = crate::chat::ask_user::validate_response(
            &prompt,
            AskUserResponseResult {
                phase: ASK_USER_PHASE_ANSWERED.into(),
                answers: HashMap::new(),
            },
        )
        .unwrap();
        assert_eq!(
            (codec.encode)(&input, &prompt, &response),
            serde_json::json!({"action":"accept","content":{}})
        );
        assert_eq!(
            (codec.encode)(&input, &prompt, &crate::chat::ask_user::skipped_response()),
            serde_json::json!({"action":"decline"})
        );
        assert_eq!(
            (codec.encode)(
                &input,
                &prompt,
                &crate::chat::ask_user::cancelled_response()
            ),
            serde_json::json!({"action":"cancel"})
        );
    }

    #[test]
    fn mcp_string_constraints_and_boolean_enum_use_the_typed_value() {
        let codec = codec_for("grok", "elicitation/create").unwrap();
        let input = serde_json::json!({"requestedSchema":{"type":"object", "properties":{
            "note":{"type":"string", "minLength":2, "maxLength":3},
            "flag":{"type":"boolean", "enum":[false,true]}}}});
        let prompt = (codec.parse)(&input).unwrap();
        let mut response = AskUserResponseResult {
            phase: ASK_USER_PHASE_ANSWERED.into(),
            answers: HashMap::from([
                (
                    "note".into(),
                    AskUserAnswer {
                        selected_option_ids: vec![],
                        custom_text: Some(" a ".into()),
                    },
                ),
                (
                    "flag".into(),
                    AskUserAnswer {
                        selected_option_ids: vec!["0".into()],
                        custom_text: None,
                    },
                ),
            ]),
        };
        let checked = crate::chat::ask_user::validate_response(&prompt, response.clone()).unwrap();
        assert_eq!(
            (codec.encode)(&input, &prompt, &checked),
            serde_json::json!({"action":"accept","content":{"note":" a ","flag":false}})
        );
        for text in ["", "a", "long"] {
            response.answers.get_mut("note").unwrap().custom_text = Some(text.into());
            assert!(
                crate::chat::ask_user::validate_response(&prompt, response.clone())
                    .unwrap_err()
                    .contains("note")
            );
        }
    }

    #[test]
    fn mcp_contract_fixture_preserves_optional_and_typed_values() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../../src/chat/fixtures/ask-user-mcp-contract.json"
        ))
        .unwrap();
        let codec = codec_for("grok", "elicitation/create").unwrap();
        let prompt = (codec.parse)(&fixture["input"]).unwrap();
        let wire = serde_json::to_value(crate::chat::protocol::ChatAskUserPromptPayload::from(
            &prompt,
        ))
        .unwrap();
        assert_eq!(wire["questions"], fixture["expectedQuestions"]);
        let response = crate::chat::ask_user::validate_response(
            &prompt,
            AskUserResponseResult {
                phase: ASK_USER_PHASE_ANSWERED.into(),
                answers: serde_json::from_value(fixture["answers"].clone()).unwrap(),
            },
        )
        .unwrap();
        assert_eq!(
            (codec.encode)(&fixture["input"], &prompt, &response),
            serde_json::json!({"action":"accept", "content":fixture["expectedContent"]})
        );
    }

    #[test]
    fn mcp_form_elicitation_round_trips_typed_content() {
        let input = serde_json::json!({
            "message": "MCP 工具需要补充信息",
            "requestedSchema": {
                "type": "object",
                "required": ["environment", "confirm", "retries"],
                "properties": {
                    "environment": {
                        "type": "string",
                        "title": "环境",
                        "enum": ["staging", "production"]
                    },
                    "confirm": { "type": "boolean", "title": "确认执行" },
                    "retries": { "type": "integer", "title": "重试次数" }
                }
            }
        });
        let prompt = parse_mcp_elicitation(&input).expect("form elicitation");
        assert_eq!(prompt.title.as_deref(), Some("MCP 工具需要补充信息"));
        assert_eq!(prompt.questions.len(), 3);
        assert_eq!(prompt.questions[0].options.len(), 2);
        let answered = AskUserResponseResult {
            phase: ASK_USER_PHASE_ANSWERED.to_string(),
            answers: HashMap::from([
                (
                    "environment".to_string(),
                    AskUserAnswer {
                        selected_option_ids: vec!["1".to_string()],
                        custom_text: None,
                    },
                ),
                (
                    "confirm".to_string(),
                    AskUserAnswer {
                        selected_option_ids: vec!["true".to_string()],
                        custom_text: None,
                    },
                ),
                (
                    "retries".to_string(),
                    AskUserAnswer {
                        selected_option_ids: vec![],
                        custom_text: Some("3".to_string()),
                    },
                ),
            ]),
        };
        let result = encode_mcp_elicitation(&input, &prompt, &answered);
        assert_eq!(result["action"], "accept");
        assert_eq!(result["content"]["environment"], "production");
        assert_eq!(result["content"]["confirm"], true);
        assert_eq!(result["content"]["retries"], 3);
        assert!(codec_for("grok", "elicitation/create").is_some());
    }

    #[test]
    fn unsupported_mcp_elicitation_shapes_are_declined_by_the_codec() {
        assert!(parse_mcp_elicitation(&serde_json::json!({
            "message": "Open this URL",
            "url": "https://example.com"
        }))
        .is_none());
        assert!(parse_mcp_elicitation(&serde_json::json!({
            "requested_schema": {
                "properties": {
                    "choice": { "enum": [1] }
                }
            }
        }))
        .is_none());
    }
}
