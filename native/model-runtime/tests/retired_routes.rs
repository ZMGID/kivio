//! Exercise the actual Rust persisted-route validator against the shared TS fixtures.
#[path = "../../../src-tauri/src/chat/route_contract.rs"]
mod route_contract;

#[test]
fn retired_works_routes_are_never_conversation_ids_or_restore_targets() {
    for route in ["#chat/artifacts", "#chat/artifacts/old?tab=files"] {
        let path = route_contract::path_from_route(route);
        assert_eq!(route_contract::decode_conversation_route_id(path), None);
        assert!(!route_contract::is_rememberable_chat_path(path));
        assert!(!route_contract::is_restorable_chat_route(route));
    }
}
