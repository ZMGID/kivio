use serde_json::Value;

pub(super) fn sanitize_api_message_for_model(message: &Value) -> Value {
    let mut sanitized = message.clone();
    if let Some(content) = sanitized.get_mut("content") {
        sanitize_api_content_for_model(content);
    }
    sanitized
}

fn sanitize_api_content_for_model(content: &mut Value) {
    match content {
        Value::String(text) => {
            *text = sanitize_image_payloads_for_model(text);
        }
        Value::Array(parts) => {
            for part in parts {
                if let Some(text) = part.get("text").and_then(|value| value.as_str()) {
                    let sanitized = sanitize_image_payloads_for_model(text);
                    if let Some(text_value) = part.get_mut("text") {
                        *text_value = Value::String(sanitized);
                    }
                }
            }
        }
        _ => {}
    }
}

pub(super) fn sanitize_image_payloads_for_model(content: &str) -> String {
    let without_data_urls = strip_image_data_urls_for_model(content);
    without_data_urls
        .lines()
        .map(|line| {
            if looks_like_inline_image_base64(line.trim()) {
                "[image base64 omitted; image is available as a tool artifact]"
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn strip_image_data_urls_for_model(content: &str) -> String {
    let mut output = String::with_capacity(content.len());
    let mut rest = content;
    while let Some(start) = rest.find("data:image/") {
        output.push_str(&rest[..start]);
        let after_start = &rest[start..];
        let Some(base64_marker) = after_start.find(";base64,") else {
            output.push_str("data:image/");
            rest = &after_start["data:image/".len()..];
            continue;
        };
        let payload_start = start + base64_marker + ";base64,".len();
        let mut payload_end = payload_start;
        for (offset, ch) in rest[payload_start..].char_indices() {
            if ch.is_ascii_alphanumeric() || matches!(ch, '+' | '/' | '=') {
                payload_end = payload_start + offset + ch.len_utf8();
            } else {
                break;
            }
        }
        output.push_str("[image data URL omitted; image is available as a tool artifact]");
        rest = &rest[payload_end..];
    }
    output.push_str(rest);
    output
}

fn looks_like_inline_image_base64(value: &str) -> bool {
    if value.len() < 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'='))
    {
        return false;
    }
    value.starts_with("iVBORw0KGgo")
        || value.starts_with("/9j/")
        || value.starts_with("R0lGOD")
        || value.starts_with("UklGR")
        || value.starts_with("PHN2Zy")
        || value.starts_with("PD94bWwg")
}
