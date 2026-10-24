use kivio_model_runtime::automation_catalog;
use serde_json::json;

#[test]
fn good_graphs_survive_bad_entries_and_unknown_fields_are_never_rewritten() {
    let root = tempfile::tempdir().unwrap();
    let good = root.path().join("good.json");
    let bytes = serde_json::to_vec(&json!({
        "schemaVersion": 1, "id": "good", "name": "Synthetic graph", "enabled": true,
        "nodes": [{"id": "manual", "type": "trigger.manual", "data": {}}],
        "edges": [], "unknownFutureField": {"preserve": true}
    }))
    .unwrap();
    std::fs::write(&good, &bytes).unwrap();
    std::fs::write(root.path().join("bad.json"), b"{invalid").unwrap();
    let (items, warnings) = automation_catalog::list_in(root.path()).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].name, "Synthetic graph");
    assert!(items[0].enabled);
    assert_eq!(warnings.len(), 1);
    assert_eq!(std::fs::read(&good).unwrap(), bytes);
    assert_eq!(
        std::fs::read(root.path().join("bad.json")).unwrap(),
        b"{invalid"
    );
}

#[test]
fn unsupported_versions_are_reported_and_missing_directory_stays_missing() {
    let root = tempfile::tempdir().unwrap();
    let absent = root.path().join("absent");
    assert!(automation_catalog::list_in(&absent).unwrap().0.is_empty());
    assert!(!absent.exists());
    std::fs::write(
        root.path().join("future.json"),
        json!({"schemaVersion": 99}).to_string(),
    )
    .unwrap();
    let (items, warnings) = automation_catalog::list_in(root.path()).unwrap();
    assert!(items.is_empty());
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("schemaVersion 99"));
}
