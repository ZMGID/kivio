//! Shared chat route contract.
//!
//! The vocabulary and cross-language fixtures live in `src/chat/routeContract.json`, which is
//! imported directly by TypeScript and embedded here at compile time. Keep parsing mechanics in
//! this module so persisted-route validation cannot grow a second allowlist in `windows.rs`.

use std::sync::OnceLock;

use serde::Deserialize;

const CONTRACT_SOURCE: &str = include_str!("../../../src/chat/routeContract.json");

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChatRouteContract {
    version: u32,
    center_segments: Vec<String>,
    rememberable_center_segments: Vec<String>,
    #[cfg(test)]
    cases: Vec<ChatRouteCase>,
}

#[cfg(test)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChatRouteCase {
    name: String,
    raw: String,
    path: String,
    kind: String,
    decoded_conversation_id: Option<String>,
    rememberable: bool,
    restorable: bool,
}

fn contract() -> &'static ChatRouteContract {
    static CONTRACT: OnceLock<ChatRouteContract> = OnceLock::new();
    CONTRACT.get_or_init(|| {
        let parsed: ChatRouteContract =
            serde_json::from_str(CONTRACT_SOURCE).expect("embedded chat route contract is valid");
        assert_eq!(parsed.version, 1, "unsupported chat route contract version");
        parsed
    })
}

pub(crate) fn path_from_route(route: &str) -> &str {
    route
        .strip_prefix('#')
        .unwrap_or(route)
        .split('?')
        .next()
        .unwrap_or("")
}

fn is_center_segment(segment: &str) -> bool {
    contract()
        .center_segments
        .iter()
        .any(|candidate| candidate == segment)
}

fn is_rememberable_center_segment(segment: &str) -> bool {
    contract()
        .rememberable_center_segments
        .iter()
        .any(|candidate| candidate == segment)
}

fn decode_percent_encoded_segment(encoded: &str) -> Option<String> {
    if encoded.is_empty() || encoded.contains('/') {
        return None;
    }
    let source = encoded.as_bytes();
    let mut decoded = Vec::with_capacity(source.len());
    let mut cursor = 0;
    while cursor < source.len() {
        if source[cursor] != b'%' {
            decoded.push(source[cursor]);
            cursor += 1;
            continue;
        }
        if cursor + 2 >= source.len() {
            return None;
        }
        let high = hex_value(source[cursor + 1])?;
        let low = hex_value(source[cursor + 2])?;
        decoded.push((high << 4) | low);
        cursor += 3;
    }
    String::from_utf8(decoded).ok()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[derive(Debug, PartialEq, Eq)]
enum RouteKind<'a> {
    Root,
    Conversation,
    Center(&'a str),
    Other,
}

fn route_kind(path: &str) -> RouteKind<'_> {
    if path == "chat" {
        return RouteKind::Root;
    }
    let Some(remainder) = path.strip_prefix("chat/") else {
        return RouteKind::Other;
    };
    let segment = remainder.split('/').next().unwrap_or("");
    if is_center_segment(segment) {
        return RouteKind::Center(segment);
    }
    if decode_percent_encoded_segment(remainder).is_some() {
        RouteKind::Conversation
    } else {
        RouteKind::Other
    }
}

pub(crate) fn decode_conversation_route_id(path: &str) -> Option<String> {
    if route_kind(path) != RouteKind::Conversation {
        return None;
    }
    decode_percent_encoded_segment(path.strip_prefix("chat/")?)
}

pub(crate) fn is_rememberable_chat_path(path: &str) -> bool {
    match route_kind(path) {
        RouteKind::Conversation => decode_conversation_route_id(path).is_some(),
        RouteKind::Center(segment) => is_rememberable_center_segment(segment),
        RouteKind::Root | RouteKind::Other => false,
    }
}

pub(crate) fn is_restorable_chat_route(route: &str) -> bool {
    let path = path_from_route(route);
    path == "chat" || is_rememberable_chat_path(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind_label(kind: RouteKind<'_>) -> &str {
        match kind {
            RouteKind::Root => "root",
            RouteKind::Conversation => "conversation",
            RouteKind::Center(segment) => segment,
            RouteKind::Other => "other",
        }
    }

    #[test]
    fn rust_codec_matches_every_shared_cross_language_fixture() {
        for fixture in &contract().cases {
            let path = path_from_route(&fixture.raw);
            assert_eq!(path, fixture.path, "{} path", fixture.name);
            assert_eq!(
                kind_label(route_kind(path)),
                fixture.kind,
                "{} kind",
                fixture.name
            );
            assert_eq!(
                decode_conversation_route_id(path),
                fixture.decoded_conversation_id,
                "{} decoded id",
                fixture.name
            );
            assert_eq!(
                is_rememberable_chat_path(path),
                fixture.rememberable,
                "{} rememberable",
                fixture.name
            );
            assert_eq!(
                is_restorable_chat_route(&fixture.raw),
                fixture.restorable,
                "{} restorable",
                fixture.name
            );
        }
    }

    #[test]
    fn rememberable_centers_are_declared_centers() {
        for segment in &contract().rememberable_center_segments {
            assert!(
                contract().center_segments.contains(segment),
                "rememberable center {segment} must also be a center"
            );
        }
    }
}
