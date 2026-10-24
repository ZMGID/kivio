pub use super::conversation_data::*;
pub use super::group_catalog::{ChatProject, ChatProjectIndex, ChatSet, ChatSetIndex};

#[cfg(test)]
mod tests {

    use super::*;

    // 多模型一问多答（任务 06-30 步骤 2）：旧会话 JSON 缺新字段时反序列化不崩（R9/AC6）。

    #[test]
    fn chat_message_deserializes_without_multi_model_fields() {
        // 旧版 assistant 消息，没有 group_id / provider_id / model。
        let json = r#"{
            "id": "msg_old",
            "role": "assistant",
            "content": "hello",
            "timestamp": 123
        }"#;
        let msg: ChatMessage = serde_json::from_str(json).expect("legacy ChatMessage parses");
        assert_eq!(msg.id, "msg_old");
        assert!(msg.group_id.is_none());
        assert!(msg.provider_id.is_none());
        assert!(msg.model.is_none());
    }

    // 会话级三态联网搜索（任务 07-23，R1）：老会话 web_search_mode 缺字段反序列化为 None，
    // 且有效模式回退全局 nativeTools.webSearch，保证旧对话行为逐字节不变。
    #[test]
    fn conversation_deserializes_without_web_search_mode_field() {
        // 只给必需字段的最小老会话 JSON（无 web_search_mode）。
        let json = r#"{
            "id": "conv_old",
            "title": "old",
            "provider_id": "openai",
            "model": "gpt-4o",
            "messages": [],
            "created_at": 1,
            "updated_at": 1
        }"#;
        let conv: Conversation =
            serde_json::from_str(json).expect("legacy Conversation parses without web_search_mode");
        assert!(conv.web_search_mode.is_none());
    }

    #[test]
    fn web_search_mode_resolve_falls_back_to_global() {
        let mut settings = crate::settings::Settings::default();
        // None（老会话/未设置）→ 依全局开关：on ⇒ ThirdParty，off ⇒ Off。
        settings.chat_tools.native_tools.web_search = true;
        assert_eq!(
            WebSearchMode::resolve(None, &settings),
            WebSearchMode::ThirdParty
        );
        settings.chat_tools.native_tools.web_search = false;
        assert_eq!(WebSearchMode::resolve(None, &settings), WebSearchMode::Off);
        // 显式设过 ⇒ 无视全局，原样返回（即便全局 off 也保留会话选择）。
        assert_eq!(
            WebSearchMode::resolve(Some(WebSearchMode::Builtin), &settings),
            WebSearchMode::Builtin
        );
        assert_eq!(
            WebSearchMode::resolve(Some(WebSearchMode::Off), &settings),
            WebSearchMode::Off
        );
        settings.chat_tools.native_tools.web_search = true;
        assert_eq!(
            WebSearchMode::resolve(Some(WebSearchMode::Off), &settings),
            WebSearchMode::Off
        );
    }

    #[test]
    fn chat_message_roundtrips_multi_model_fields() {
        let json = r#"{
            "id": "msg_new",
            "role": "assistant",
            "content": "hi",
            "group_id": "grp_1",
            "provider_id": "openai",
            "model": "gpt-4o",
            "timestamp": 9
        }"#;
        let msg: ChatMessage = serde_json::from_str(json).expect("new ChatMessage parses");
        assert_eq!(msg.group_id.as_deref(), Some("grp_1"));
        assert_eq!(msg.provider_id.as_deref(), Some("openai"));
        assert_eq!(msg.model.as_deref(), Some("gpt-4o"));
        // 重新序列化再反序列化保持一致。
        let again: ChatMessage =
            serde_json::from_str(&serde_json::to_string(&msg).unwrap()).unwrap();
        assert_eq!(again.group_id, msg.group_id);
        assert_eq!(again.model, msg.model);
    }

    #[test]
    fn conversation_deserializes_without_multi_model_fields() {
        // 旧版会话，没有 reply_models / group_selections。
        let json = r#"{
            "id": "conv_old",
            "title": "t",
            "provider_id": "p",
            "model": "m",
            "messages": [],
            "created_at": 1,
            "updated_at": 2
        }"#;
        let conv: Conversation = serde_json::from_str(json).expect("legacy Conversation parses");
        assert_eq!(conv.id, "conv_old");
        assert!(conv.reply_models.is_empty());
        assert!(conv.group_selections.is_empty());
    }

    #[test]
    fn conversation_roundtrips_multi_model_fields() {
        let json = r#"{
            "id": "conv_new",
            "title": "t",
            "provider_id": "p",
            "model": "m",
            "messages": [],
            "reply_models": [
                {"provider_id": "openai", "model": "gpt-4o"},
                {"provider_id": "anthropic", "model": "claude-3"}
            ],
            "group_selections": {"grp_1": "msg_a"},
            "created_at": 1,
            "updated_at": 2
        }"#;
        let conv: Conversation = serde_json::from_str(json).expect("new Conversation parses");
        assert_eq!(conv.reply_models.len(), 2);
        assert_eq!(conv.reply_models[0].provider_id, "openai");
        assert_eq!(conv.reply_models[0].model, "gpt-4o");
        assert_eq!(
            conv.group_selections.get("grp_1").map(String::as_str),
            Some("msg_a")
        );
    }
}
