//! Run the M1 thin rebuild against a REAL whale snapshot (read-only: loads a
//! snapshot file, rebuilds, reports accounting — writes nothing anywhere).
//! Usage: cargo run -p zeron-doc --example rebuild_whale -- <snapshot.bin>
use loro::LoroDoc;
use zeron_doc::rebuild::{doc_epoch, rebuild_thin_doc};
use zeron_doc::schema::SessionDoc;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: rebuild_whale <snapshot.bin>");
    let bytes = std::fs::read(&path).expect("read snapshot");
    let doc = LoroDoc::new();
    doc.import(&bytes).expect("import snapshot");
    let source = SessionDoc::from_doc(doc);

    let entries = source.read_entries().expect("read entries").len();
    let commands = source.read_commands().expect("read commands").len();
    let src_epoch = doc_epoch(&source);
    let start = std::time::Instant::now();
    let rebuilt = rebuild_thin_doc(&source).expect("rebuild");
    let elapsed = start.elapsed();
    let thin_snapshot = rebuilt.doc.export_snapshot().expect("export");

    // Idempotence probe: rebuilding the REBUILT doc owes the sidecar nothing.
    let again = rebuild_thin_doc(&rebuilt.doc).expect("re-rebuild");

    if let Some(out_prefix) = std::env::args().nth(2) {
        std::fs::write(format!("{out_prefix}.thin.bin"), &thin_snapshot).unwrap();
        std::fs::write(
            format!("{out_prefix}.frontier.bin"),
            rebuilt.doc.doc().oplog_vv().encode(),
        )
        .unwrap();
    }
    println!(
        "RESULT:{}",
        serde_json::json!({
            "sourceBytes": bytes.len(),
            "sourceEntries": entries,
            "sourceCommands": commands,
            "sourceEpoch": src_epoch,
            "thinBytes": thin_snapshot.len(),
            "thinEntries": rebuilt.entries,
            "commandsCopied": rebuilt.commands_copied,
            "sidecarPayloads": rebuilt.sidecar.len(),
            "sidecarBytes": rebuilt
                .sidecar
                .iter()
                .map(|p| p.output.as_deref().map_or(0, str::len))
                .sum::<usize>(),
            "thinEpoch": doc_epoch(&rebuilt.doc),
            "rebuildMs": elapsed.as_millis(),
            "secondPassSidecar": again.sidecar.len(),
            "shrink": format!("{:.1}x", bytes.len() as f64 / thin_snapshot.len() as f64),
        })
    );
}
