// Shared bounded request-debug owner. Redaction remains in request_debug_data.
use std::{
    collections::VecDeque,
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};
/// Max records kept in the ring buffer. Oldest are evicted first.
pub const REQUEST_DEBUG_CAPACITY: usize = 50;

/// Owns memory and the existing best-effort disk mirror. Mutation ordering is
/// mirror gate -> buffer; release buffer before disk I/O. No await is performed.
pub(crate) struct RequestDebugState {
    buffer: Mutex<VecDeque<crate::chat::request_debug::RequestDebugRecord>>,
    mirror_gate: Mutex<()>,
    path: PathBuf,
}

impl RequestDebugState {
    pub(crate) fn new(usage_dir: &Path) -> Self {
        let root = usage_dir.parent().unwrap_or(usage_dir);
        Self {
            buffer: Mutex::new(VecDeque::new()),
            mirror_gate: Mutex::new(()),
            path: root.join("request_debug").join("records.jsonl"),
        }
    }

    pub(crate) fn record(&self, record: crate::chat::request_debug::RequestDebugRecord) {
        let _mirror = self.mirror_gate.lock().unwrap_or_else(|e| e.into_inner());
        let mut buffer = self.buffer.lock().unwrap_or_else(|e| e.into_inner());
        while buffer.len() >= REQUEST_DEBUG_CAPACITY {
            buffer.pop_front();
        }
        buffer.push_back(record);
        let disk_view: Vec<_> = buffer.iter().rev().cloned().collect();
        drop(buffer);
        persist_to_disk(&self.path, &disk_view);
    }

    pub(crate) fn snapshot(&self) -> Vec<crate::chat::request_debug::RequestDebugRecord> {
        self.buffer
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .rev()
            .cloned()
            .collect()
    }

    pub(crate) fn clear(&self) {
        let _mirror = self.mirror_gate.lock().unwrap_or_else(|e| e.into_inner());
        self.buffer
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        let _ = fs::remove_file(&self.path);
    }
}

/// Rewrite the on-disk mirror with `records` (newest-first, one JSON per line).
/// Best-effort: a debug convenience must never break the request path, so all
/// IO errors are swallowed.
fn persist_to_disk(path: &Path, records: &[crate::chat::request_debug::RequestDebugRecord]) {
    if let Some(dir) = path.parent() {
        if fs::create_dir_all(dir).is_err() {
            return;
        }
    }
    let mut out = String::with_capacity(records.len() * 512);
    for record in records {
        if let Ok(line) = serde_json::to_string(record) {
            out.push_str(&line);
            out.push('\n');
        }
    }
    let _ = fs::write(path, out);
}
