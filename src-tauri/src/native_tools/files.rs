use std::{
    collections::HashSet,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{Condvar, Mutex, OnceLock},
    time::UNIX_EPOCH,
};

use serde::Serialize;
use serde_json::{json, Value};
use uuid::Uuid;

use super::{
    assert_writable_path, resolve_tool_read_path, resolve_tool_write_entry_path,
    resolve_tool_write_path, workspace_display_path, NativeToolWorkspace, MAX_READ_FILE_BYTES,
};

const MAX_LIST_ENTRIES: usize = 500;
const MAX_GLOB_RESULTS: usize = 500;
const MAX_SEARCH_FILES: usize = 2_000;
const MAX_SEARCH_MATCHES: usize = 200;
const MAX_SEARCH_FILE_BYTES: u64 = 1024 * 1024;
const UTF8_BOM: &str = "\u{feff}";
const DEFAULT_IGNORED_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    "build",
    ".next",
    ".turbo",
    ".vite",
];

static FILE_MUTATION_LOCKS: OnceLock<FileMutationLocks> = OnceLock::new();

#[derive(Debug, Clone, Serialize)]
pub struct FileMutationResult {
    pub ok: bool,
    pub operation: String,
    pub target_touched: bool,
    pub resolved_path: Option<String>,
    pub files: Vec<FileMutationFile>,
    pub bytes_written: u64,
    pub additions: usize,
    pub removals: usize,
    pub diff: String,
    pub warnings: Vec<String>,
    pub diagnostics: Vec<Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReadFileResult {
    pub path: String,
    pub resolved_path: String,
    pub content: String,
    pub total_lines: usize,
    pub start_line: usize,
    pub end_line: usize,
    pub truncated: bool,
    pub file_size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_offset: Option<usize>,
    pub read_state: ReadFileState,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReadFileState {
    pub scope: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mtime: Option<u64>,
    pub already_read: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileMutationFile {
    pub path: String,
    pub operation: String,
    pub bytes_written: u64,
    pub additions: usize,
    pub removals: usize,
    pub diff: String,
}

impl FileMutationResult {
    pub fn summary(&self) -> String {
        let stats = format!("+{} -{}", self.additions, self.removals);
        if let Some(file) = self.files.first().filter(|_| self.files.len() == 1) {
            return format!(
                "{} {} ({stats})",
                mutation_operation_label(&file.operation),
                file.path
            );
        }
        format!(
            "{} {} file(s) ({stats})",
            mutation_operation_label(&self.operation),
            self.files.len()
        )
    }
}

pub fn read_file(
    workspace: &NativeToolWorkspace,
    arguments: &Value,
) -> Result<ReadFileResult, String> {
    let path = arguments
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "read_file requires path".to_string())?;
    let full = resolve_tool_read_path(workspace, path)?;
    if !full.is_file() {
        return Err(format!("不是可读取的文件: {path}"));
    }
    let metadata = fs::metadata(&full).map_err(|err| format!("Read metadata failed: {err}"))?;

    let offset = arguments
        .get("offset")
        .and_then(|v| v.as_u64())
        .unwrap_or(1)
        .max(1) as usize;
    let limit = arguments
        .get("limit")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize);

    if metadata.len() > MAX_READ_FILE_BYTES {
        if arguments.get("offset").is_none() && limit.is_none() {
            return Err(format!(
                "File too large to read at once ({} bytes, max {}). Pass offset/limit to read a line window.",
                metadata.len(),
                MAX_READ_FILE_BYTES
            ));
        }
        return read_file_window_streaming(workspace, &full, &metadata, offset, limit);
    }

    let content = fs::read_to_string(&full).map_err(|err| format!("Read file failed: {err}"))?;

    let lines: Vec<&str> = content.lines().collect();
    let total_lines = lines.len();
    let start = offset.saturating_sub(1).min(lines.len());
    let end = limit
        .map(|lim| (start + lim).min(lines.len()))
        .unwrap_or(lines.len());
    let truncated = end < total_lines;
    let returned_content = if offset == 1 && limit.is_none() {
        content
    } else {
        lines[start..end].join("\n")
    };
    let scope = if offset == 1 && !truncated {
        "full"
    } else {
        "partial"
    };
    Ok(ReadFileResult {
        path: workspace_display_path(workspace, &full),
        resolved_path: full.display().to_string(),
        content: returned_content,
        total_lines,
        start_line: if total_lines == 0 { 0 } else { start + 1 },
        end_line: end,
        truncated,
        file_size: metadata.len(),
        next_offset: truncated.then_some(end + 1),
        read_state: ReadFileState {
            scope: scope.to_string(),
            mtime: file_mtime_secs(&metadata),
            already_read: false,
        },
        warnings: Vec::new(),
    })
}

/// Default line window when reading an oversized file with offset but no limit.
const LARGE_READ_DEFAULT_LIMIT: usize = 2_000;

/// Streams an oversized file line by line, keeping only the requested window in
/// memory. The window itself is still capped at MAX_READ_FILE_BYTES.
fn read_file_window_streaming(
    workspace: &NativeToolWorkspace,
    full: &Path,
    metadata: &fs::Metadata,
    offset: usize,
    limit: Option<usize>,
) -> Result<ReadFileResult, String> {
    use std::io::BufRead;

    let limit = limit.unwrap_or(LARGE_READ_DEFAULT_LIMIT).max(1);
    let start = offset.saturating_sub(1);
    let end = start.saturating_add(limit);

    let file = fs::File::open(full).map_err(|err| format!("Read file failed: {err}"))?;
    let reader = std::io::BufReader::new(file);
    let mut window: Vec<String> = Vec::new();
    let mut window_bytes = 0usize;
    let mut window_byte_capped = false;
    let mut total_lines = 0usize;
    for line in reader.lines() {
        let line = line.map_err(|err| format!("Read file failed: {err}"))?;
        if total_lines >= start && total_lines < end && !window_byte_capped {
            if window_bytes + line.len() > MAX_READ_FILE_BYTES as usize {
                window_byte_capped = true;
            } else {
                window_bytes += line.len();
                window.push(line);
            }
        }
        total_lines += 1;
    }

    let start = start.min(total_lines);
    let end = (start + window.len()).min(total_lines);
    let truncated = end < total_lines;
    let mut warnings = Vec::new();
    if window_byte_capped {
        warnings.push(format!(
            "Line window exceeded {MAX_READ_FILE_BYTES} bytes; returned fewer lines than requested. Continue with offset={}.",
            end + 1
        ));
    }
    Ok(ReadFileResult {
        path: workspace_display_path(workspace, full),
        resolved_path: full.display().to_string(),
        content: window.join("\n"),
        total_lines,
        start_line: if total_lines == 0 { 0 } else { start + 1 },
        end_line: end,
        truncated,
        file_size: metadata.len(),
        next_offset: truncated.then_some(end + 1),
        read_state: ReadFileState {
            scope: "partial".to_string(),
            mtime: file_mtime_secs(metadata),
            already_read: false,
        },
        warnings,
    })
}

fn file_mtime_secs(metadata: &fs::Metadata) -> Option<u64> {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
}

pub fn write_file(
    workspace: &NativeToolWorkspace,
    arguments: &Value,
) -> Result<FileMutationResult, String> {
    let path = arguments
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "write_file requires path".to_string())?;
    let content = arguments
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "write_file requires content".to_string())?;
    let full = resolve_tool_write_path(workspace, path)?;
    if !workspace.has_project() {
        assert_writable_path(&full)?;
    }
    let _guard = acquire_file_mutation_locks([full.clone()])?;
    let existed = full.is_file();
    // Placeholder phrases ("rest of file unchanged", "省略") are a real hazard
    // only when a model lazily overwrites existing code; prose like meeting
    // notes legitimately contains them, so new files and non-code files pass.
    if existed && is_code_like_path(&full) && looks_like_placeholder_content(content) {
        return Err("write_file rejected placeholder/lazy content; target untouched".to_string());
    }
    // The existing content is only needed for the diff; degrade gracefully on non-UTF-8.
    let before = if existed {
        fs::read_to_string(&full).ok()
    } else {
        None
    };
    atomic_write_text(&full, content, before.as_deref())
        .map_err(|err| format!("Write file failed: {err}"))?;
    let operation = if existed { "overwrite" } else { "create" };
    let diff_omitted = existed && before.is_none();
    let file = if diff_omitted {
        FileMutationFile {
            path: workspace_display_path(workspace, &full),
            operation: operation.to_string(),
            bytes_written: content.len() as u64,
            additions: 0,
            removals: 0,
            diff: String::new(),
        }
    } else {
        planned_file_result(workspace, full, operation, before.as_deref(), Some(content))?
    };
    let mut result = file_mutation_result(operation, vec![file]);
    if diff_omitted {
        result
            .warnings
            .push("Existing file content is not valid UTF-8; diff omitted.".to_string());
    }
    Ok(result)
}

pub fn edit_file(
    workspace: &NativeToolWorkspace,
    arguments: &Value,
) -> Result<FileMutationResult, String> {
    let path = arguments
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "edit_file requires path".to_string())?;
    let old_string = arguments
        .get("old_string")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "edit_file requires old_string".to_string())?;
    let new_string = arguments
        .get("new_string")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "edit_file requires new_string".to_string())?;
    let replace_all = arguments
        .get("replace_all")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let full = resolve_tool_write_path(workspace, path)?;
    if !workspace.has_project() {
        assert_writable_path(&full)?;
    }
    let _guard = acquire_file_mutation_locks([full.clone()])?;
    if !full.is_file() {
        return Err(format!("不是可编辑的文件: {path}"));
    }

    let content = fs::read_to_string(&full).map_err(|err| format!("Read file failed: {err}"))?;
    // 行尾归一后再匹配：模型给的 old_string 通常是 LF，而文件可能是 CRLF（Windows 高频），
    // 直接字面 `contains` 会 0 命中。统一归一到 LF 做匹配/计数/替换；写回时
    // atomic_write_text 依据原文件把 LF 还原成 CRLF（并保留 BOM），磁盘行尾风格不变。
    // diff 用归一后的 before 比对，避免 CRLF→LF 让每行都被算成变更。
    let normalized_content = normalize_line_endings(&content, "\n");
    let normalized_old = normalize_line_endings(old_string, "\n");
    let normalized_new = normalize_line_endings(new_string, "\n");

    if normalized_old == normalized_new {
        let display_path = workspace_display_path(workspace, &full);
        return Ok(FileMutationResult {
            ok: true,
            operation: "edit".to_string(),
            target_touched: false,
            resolved_path: Some(display_path.clone()),
            files: vec![FileMutationFile {
                path: display_path,
                operation: "noop".to_string(),
                bytes_written: content.len() as u64,
                additions: 0,
                removals: 0,
                diff: String::new(),
            }],
            bytes_written: content.len() as u64,
            additions: 0,
            removals: 0,
            diff: String::new(),
            warnings: vec!["old_string and new_string are identical; no changes made.".to_string()],
            diagnostics: Vec::new(),
        });
    }
    if !normalized_content.contains(&normalized_old) {
        return Err(
            "old_string not found in file. Re-read the file and copy an exact, contiguous snippet \
             including its leading whitespace/indentation. Line endings are normalized \
             automatically, so a CRLF vs LF mismatch is not the cause."
                .to_string(),
        );
    }
    let count = normalized_content.matches(&normalized_old).count();
    if !replace_all && count > 1 {
        return Err(format!(
            "old_string appears {count} times; set replace_all=true, or extend old_string with \
             surrounding context so it matches exactly one location."
        ));
    }

    let updated = if replace_all {
        normalized_content.replace(&normalized_old, &normalized_new)
    } else {
        normalized_content.replacen(&normalized_old, &normalized_new, 1)
    };
    atomic_write_text(&full, &updated, Some(&content))
        .map_err(|err| format!("Write file failed: {err}"))?;
    Ok(file_mutation_result(
        "edit",
        vec![planned_file_result(
            workspace,
            full,
            "edit",
            Some(&normalized_content),
            Some(&updated),
        )?],
    ))
}

struct FileMutationLocks {
    active: Mutex<HashSet<PathBuf>>,
    ready: Condvar,
}

struct FileMutationLockGuard {
    paths: Vec<PathBuf>,
}

impl Drop for FileMutationLockGuard {
    fn drop(&mut self) {
        let Some(locks) = FILE_MUTATION_LOCKS.get() else {
            return;
        };
        let Ok(mut active) = locks.active.lock() else {
            return;
        };
        for path in &self.paths {
            active.remove(path);
        }
        locks.ready.notify_all();
    }
}

fn acquire_file_mutation_locks<I>(paths: I) -> Result<FileMutationLockGuard, String>
where
    I: IntoIterator<Item = PathBuf>,
{
    let locks = FILE_MUTATION_LOCKS.get_or_init(|| FileMutationLocks {
        active: Mutex::new(HashSet::new()),
        ready: Condvar::new(),
    });
    let mut paths = paths.into_iter().collect::<Vec<_>>();
    paths.sort();
    paths.dedup();
    let mut active = locks
        .active
        .lock()
        .map_err(|_| "File mutation lock is unavailable".to_string())?;
    while paths.iter().any(|path| active.contains(path)) {
        active = locks
            .ready
            .wait(active)
            .map_err(|_| "File mutation lock is unavailable".to_string())?;
    }
    for path in &paths {
        active.insert(path.clone());
    }
    Ok(FileMutationLockGuard { paths })
}

fn file_mutation_result(operation: &str, files: Vec<FileMutationFile>) -> FileMutationResult {
    let resolved_path = files
        .first()
        .filter(|_| files.len() == 1)
        .map(|file| file.path.clone());
    let bytes_written = files.iter().map(|file| file.bytes_written).sum();
    let additions = files.iter().map(|file| file.additions).sum();
    let removals = files.iter().map(|file| file.removals).sum();
    let diff = files
        .iter()
        .map(|file| file.diff.as_str())
        .filter(|diff| !diff.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    FileMutationResult {
        ok: true,
        operation: operation.to_string(),
        target_touched: true,
        resolved_path,
        files,
        bytes_written,
        additions,
        removals,
        diff,
        warnings: Vec::new(),
        diagnostics: Vec::new(),
    }
}

fn atomic_write_text(
    target: &Path,
    content: &str,
    existing_text: Option<&str>,
) -> Result<(), String> {
    let mut text = content.to_string();
    if let Some(existing) = existing_text {
        if existing.contains("\r\n") {
            text = normalize_line_endings(&text, "\r\n");
        }
        if existing.starts_with(UTF8_BOM) && !text.starts_with(UTF8_BOM) {
            text = format!("{UTF8_BOM}{text}");
        }
    }
    atomic_write_bytes(target, text.as_bytes(), existing_text)
}

fn looks_like_placeholder_content(content: &str) -> bool {
    let normalized = content.to_ascii_lowercase();
    [
        "original code here",
        "rest of file unchanged",
        "same as before",
        "remaining code unchanged",
        "unchanged code",
        "原代码",
        "其余不变",
        "保持不变",
        "省略",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
}

/// Extensions where placeholder phrases indicate a lazily truncated overwrite
/// rather than legitimate document text.
fn is_code_like_path(path: &Path) -> bool {
    let Some(ext) = path.extension().and_then(|ext| ext.to_str()) else {
        return false;
    };
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "rs" | "ts"
            | "tsx"
            | "js"
            | "jsx"
            | "mjs"
            | "cjs"
            | "py"
            | "rb"
            | "go"
            | "java"
            | "kt"
            | "swift"
            | "c"
            | "h"
            | "cpp"
            | "hpp"
            | "cc"
            | "cs"
            | "php"
            | "sh"
            | "bash"
            | "zsh"
            | "fish"
            | "ps1"
            | "sql"
            | "html"
            | "css"
            | "scss"
            | "less"
            | "vue"
            | "svelte"
            | "json"
            | "yaml"
            | "yml"
            | "toml"
            | "xml"
            | "lua"
            | "zig"
            | "dart"
            | "scala"
    )
}

fn normalize_line_endings(content: &str, target: &str) -> String {
    let lf = content.replace("\r\n", "\n").replace('\r', "\n");
    if target == "\r\n" {
        lf.replace('\n', "\r\n")
    } else {
        lf
    }
}

fn atomic_write_bytes(
    target: &Path,
    bytes: &[u8],
    existing_text: Option<&str>,
) -> Result<(), String> {
    let parent = target
        .parent()
        .ok_or_else(|| "Target path has no parent directory".to_string())?;
    fs::create_dir_all(parent).map_err(|err| format!("Create parent dirs failed: {err}"))?;
    let file_name = target
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("file");
    let tmp = parent.join(format!(
        ".kivio-tmp-{}-{file_name}",
        Uuid::new_v4().simple()
    ));
    {
        let mut file =
            fs::File::create(&tmp).map_err(|err| format!("Create temp file failed: {err}"))?;
        file.write_all(bytes)
            .map_err(|err| format!("Write temp file failed: {err}"))?;
        file.sync_all()
            .map_err(|err| format!("Sync temp file failed: {err}"))?;
    }
    #[cfg(unix)]
    if target.exists() {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(metadata) = fs::metadata(target) {
            let _ = fs::set_permissions(
                &tmp,
                fs::Permissions::from_mode(metadata.permissions().mode()),
            );
        }
    }
    let _ = existing_text;
    #[cfg(target_os = "windows")]
    if target.exists() {
        // std::fs::rename cannot replace an existing file on Windows. This
        // fallback has a tiny non-atomic gap, but still guarantees chunks are
        // never streamed directly into the target. A future Windows API
        // ReplaceFileW path can tighten this last commit step.
        fs::remove_file(target)
            .map_err(|err| format!("Remove existing target failed before replace: {err}"))?;
    }
    match fs::rename(&tmp, target) {
        Ok(()) => Ok(()),
        Err(err) => {
            let _ = fs::remove_file(&tmp);
            Err(format!("Rename temp file failed: {err}"))
        }
    }
}

fn planned_file_result(
    workspace: &NativeToolWorkspace,
    path: PathBuf,
    operation: &str,
    before: Option<&str>,
    after: Option<&str>,
) -> Result<FileMutationFile, String> {
    let display_path = workspace_display_path(workspace, &path);
    let (diff, additions, removals) = unified_diff(&display_path, before, after);
    Ok(FileMutationFile {
        path: display_path,
        operation: operation.to_string(),
        bytes_written: after.map(|content| content.len() as u64).unwrap_or(0),
        additions,
        removals,
        diff,
    })
}

/// LCS guard: above this many DP cells for the changed middle region, fall back
/// to a coarse single-hunk diff (whole middle as remove+add).
const DIFF_LCS_MAX_CELLS: usize = 250_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiffOpKind {
    Equal,
    Remove,
    Add,
}

#[derive(Debug)]
struct DiffOp<'a> {
    kind: DiffOpKind,
    text: &'a str,
}

fn unified_diff(path: &str, before: Option<&str>, after: Option<&str>) -> (String, usize, usize) {
    let old_lines = diff_lines(before.unwrap_or(""));
    let new_lines = diff_lines(after.unwrap_or(""));
    if old_lines == new_lines {
        return (String::new(), 0, 0);
    }

    let mut prefix = 0usize;
    while prefix < old_lines.len()
        && prefix < new_lines.len()
        && old_lines[prefix] == new_lines[prefix]
    {
        prefix += 1;
    }

    let mut old_end = old_lines.len();
    let mut new_end = new_lines.len();
    while old_end > prefix && new_end > prefix && old_lines[old_end - 1] == new_lines[new_end - 1] {
        old_end -= 1;
        new_end -= 1;
    }

    let ops = build_diff_ops(&old_lines, &new_lines, prefix, old_end, new_end);

    let changed: Vec<usize> = ops
        .iter()
        .enumerate()
        .filter(|(_, op)| op.kind != DiffOpKind::Equal)
        .map(|(idx, _)| idx)
        .collect();
    if changed.is_empty() {
        return (String::new(), 0, 0);
    }

    // Prefix sums of old/new lines consumed before each op index.
    let mut old_consumed = vec![0usize; ops.len() + 1];
    let mut new_consumed = vec![0usize; ops.len() + 1];
    for (idx, op) in ops.iter().enumerate() {
        old_consumed[idx + 1] = old_consumed[idx]
            + usize::from(matches!(op.kind, DiffOpKind::Equal | DiffOpKind::Remove));
        new_consumed[idx + 1] =
            new_consumed[idx] + usize::from(matches!(op.kind, DiffOpKind::Equal | DiffOpKind::Add));
    }

    // Group changed ops into hunks: runs separated by 7+ unchanged lines split.
    let mut groups: Vec<(usize, usize)> = Vec::new();
    let mut group_start = changed[0];
    let mut group_last = changed[0];
    for &idx in &changed[1..] {
        if idx - group_last - 1 >= 7 {
            groups.push((group_start, group_last));
            group_start = idx;
        }
        group_last = idx;
    }
    groups.push((group_start, group_last));

    let old_header = if before.is_none() {
        "/dev/null".to_string()
    } else {
        format!("a/{path}")
    };
    let new_header = if after.is_none() {
        "/dev/null".to_string()
    } else {
        format!("b/{path}")
    };
    let mut out = String::new();
    out.push_str(&format!("--- {old_header}\n+++ {new_header}\n"));
    let mut additions = 0usize;
    let mut removals = 0usize;
    for (first, last) in groups {
        let hunk_start = first.saturating_sub(3);
        let hunk_end = (last + 4).min(ops.len());
        let old_count = old_consumed[hunk_end] - old_consumed[hunk_start];
        let new_count = new_consumed[hunk_end] - new_consumed[hunk_start];
        let old_start = if old_count == 0 {
            old_consumed[hunk_start]
        } else {
            old_consumed[hunk_start] + 1
        };
        let new_start = if new_count == 0 {
            new_consumed[hunk_start]
        } else {
            new_consumed[hunk_start] + 1
        };
        out.push_str(&format!(
            "@@ -{old_start},{old_count} +{new_start},{new_count} @@\n"
        ));
        for op in &ops[hunk_start..hunk_end] {
            match op.kind {
                DiffOpKind::Equal => out.push_str(&format!(" {}\n", op.text)),
                DiffOpKind::Remove => {
                    removals += 1;
                    out.push_str(&format!("-{}\n", op.text));
                }
                DiffOpKind::Add => {
                    additions += 1;
                    out.push_str(&format!("+{}\n", op.text));
                }
            }
        }
    }
    (out, additions, removals)
}

fn build_diff_ops<'a>(
    old_lines: &'a [String],
    new_lines: &'a [String],
    prefix: usize,
    old_end: usize,
    new_end: usize,
) -> Vec<DiffOp<'a>> {
    let mut ops = Vec::with_capacity(old_lines.len() + new_lines.len());
    for line in &old_lines[..prefix] {
        ops.push(DiffOp {
            kind: DiffOpKind::Equal,
            text: line,
        });
    }
    let middle_old = &old_lines[prefix..old_end];
    let middle_new = &new_lines[prefix..new_end];
    if middle_old.len().saturating_mul(middle_new.len()) > DIFF_LCS_MAX_CELLS {
        // Coarse fallback: whole middle as remove+add in a single block.
        for line in middle_old {
            ops.push(DiffOp {
                kind: DiffOpKind::Remove,
                text: line,
            });
        }
        for line in middle_new {
            ops.push(DiffOp {
                kind: DiffOpKind::Add,
                text: line,
            });
        }
    } else {
        let m = middle_old.len();
        let n = middle_new.len();
        let width = n + 1;
        let mut dp = vec![0u32; (m + 1) * width];
        for i in (0..m).rev() {
            for j in (0..n).rev() {
                dp[i * width + j] = if middle_old[i] == middle_new[j] {
                    dp[(i + 1) * width + j + 1] + 1
                } else {
                    dp[(i + 1) * width + j].max(dp[i * width + j + 1])
                };
            }
        }
        let (mut i, mut j) = (0usize, 0usize);
        while i < m && j < n {
            if middle_old[i] == middle_new[j] {
                ops.push(DiffOp {
                    kind: DiffOpKind::Equal,
                    text: &middle_old[i],
                });
                i += 1;
                j += 1;
            } else if dp[(i + 1) * width + j] >= dp[i * width + j + 1] {
                ops.push(DiffOp {
                    kind: DiffOpKind::Remove,
                    text: &middle_old[i],
                });
                i += 1;
            } else {
                ops.push(DiffOp {
                    kind: DiffOpKind::Add,
                    text: &middle_new[j],
                });
                j += 1;
            }
        }
        while i < m {
            ops.push(DiffOp {
                kind: DiffOpKind::Remove,
                text: &middle_old[i],
            });
            i += 1;
        }
        while j < n {
            ops.push(DiffOp {
                kind: DiffOpKind::Add,
                text: &middle_new[j],
            });
            j += 1;
        }
    }
    for line in &old_lines[old_end..] {
        ops.push(DiffOp {
            kind: DiffOpKind::Equal,
            text: line,
        });
    }
    ops
}

fn diff_lines(content: &str) -> Vec<String> {
    if content.is_empty() {
        return Vec::new();
    }
    let normalized = content.replace("\r\n", "\n");
    let mut lines = normalized
        .split('\n')
        .map(str::to_string)
        .collect::<Vec<_>>();
    if normalized.ends_with('\n') {
        lines.pop();
    }
    lines
}

fn mutation_operation_label(operation: &str) -> &'static str {
    match operation {
        "create" => "Created",
        "overwrite" => "Overwrote",
        "edit" => "Updated",
        "delete" => "Deleted",
        "noop" => "No changes",
        _ => "Changed",
    }
}

pub fn list_dir(workspace: &NativeToolWorkspace, arguments: &Value) -> Result<String, String> {
    let path = arguments
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or(".");
    let include_hidden = arguments
        .get("include_hidden")
        .or_else(|| arguments.get("includeHidden"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let max_entries = arguments
        .get("max_entries")
        .or_else(|| arguments.get("maxEntries"))
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .unwrap_or(200)
        .clamp(1, MAX_LIST_ENTRIES);

    let dir = resolve_tool_read_path(workspace, path)?;
    if !dir.is_dir() {
        return Err(format!("不是可列出的文件夹: {path}"));
    }

    let mut entries = Vec::new();
    for entry in fs::read_dir(&dir).map_err(|err| format!("Read directory failed: {err}"))? {
        let entry = entry.map_err(|err| format!("Read directory entry failed: {err}"))?;
        let path = entry.path();
        if !include_hidden && is_hidden_path(&path) {
            continue;
        }
        let metadata = entry
            .metadata()
            .map_err(|err| format!("Read entry metadata failed: {err}"))?;
        entries.push(path_info(workspace, &path, &metadata)?);
    }

    entries.sort_by(|a, b| {
        a.get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .cmp(b.get("type").and_then(|v| v.as_str()).unwrap_or(""))
            .then_with(|| {
                a.get("path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .cmp(b.get("path").and_then(|v| v.as_str()).unwrap_or(""))
            })
    });
    let truncated = entries.len() > max_entries;
    entries.truncate(max_entries);

    format_json(json!({
        "path": workspace_display_path(workspace, &dir),
        "entries": entries,
        "truncated": truncated
    }))
}

pub fn stat_path(workspace: &NativeToolWorkspace, arguments: &Value) -> Result<String, String> {
    let path = required_string(arguments, "path")?;
    let full = resolve_tool_read_path(workspace, path)?;
    let metadata = fs::metadata(&full).map_err(|err| format!("Read metadata failed: {err}"))?;
    format_json(path_info(workspace, &full, &metadata)?)
}

pub fn create_dir(workspace: &NativeToolWorkspace, arguments: &Value) -> Result<String, String> {
    let path = required_string(arguments, "path")?;
    let full = resolve_tool_write_path(workspace, path)?;
    if !workspace.has_project() {
        assert_writable_path(&full)?;
    }
    fs::create_dir_all(&full).map_err(|err| format!("Create directory failed: {err}"))?;
    Ok(format!(
        "Created directory {}",
        workspace_display_path(workspace, &full)
    ))
}

pub fn delete_path(workspace: &NativeToolWorkspace, arguments: &Value) -> Result<String, String> {
    let path = required_string(arguments, "path")?;
    let recursive = arguments
        .get("recursive")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let full = resolve_tool_write_entry_path(workspace, path)?;
    if !workspace.has_project() {
        assert_writable_path(&full)?;
    }
    let _guard = acquire_file_mutation_locks([full.clone()])?;
    reject_workspace_root_delete(workspace, &full)?;

    let metadata = fs::symlink_metadata(&full).map_err(|_| format!("路径不存在: {path}"))?;
    let file_type = metadata.file_type();
    if file_type.is_symlink() || metadata.is_file() {
        fs::remove_file(&full).map_err(|err| format!("Delete file failed: {err}"))?;
    } else if metadata.is_dir() {
        if recursive {
            fs::remove_dir_all(&full).map_err(|err| format!("Delete directory failed: {err}"))?;
        } else {
            fs::remove_dir(&full).map_err(|err| format!("Delete directory failed: {err}"))?;
        }
    } else {
        return Err(format!("不是可删除的文件或文件夹: {path}"));
    }

    Ok(format!(
        "Deleted {}",
        workspace_display_path(workspace, &full)
    ))
}

pub fn move_path(workspace: &NativeToolWorkspace, arguments: &Value) -> Result<String, String> {
    let from = required_string(arguments, "from")?;
    let to = required_string(arguments, "to")?;
    let source = resolve_tool_write_entry_path(workspace, from)?;
    let destination = resolve_tool_write_path(workspace, to)?;
    if !workspace.has_project() {
        assert_writable_path(&source)?;
        assert_writable_path(&destination)?;
    }
    let _guard = acquire_file_mutation_locks([source.clone(), destination.clone()])?;
    reject_workspace_root_delete(workspace, &source)?;
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|err| format!("Create parent dirs failed: {err}"))?;
    }
    fs::rename(&source, &destination).map_err(|err| format!("Move path failed: {err}"))?;
    Ok(format!(
        "Moved {} to {}",
        workspace_display_path(workspace, &source),
        workspace_display_path(workspace, &destination)
    ))
}

pub fn copy_path(workspace: &NativeToolWorkspace, arguments: &Value) -> Result<String, String> {
    let from = required_string(arguments, "from")?;
    let to = required_string(arguments, "to")?;
    let source = resolve_tool_read_path(workspace, from)?;
    let destination = resolve_tool_write_path(workspace, to)?;
    if !workspace.has_project() {
        assert_writable_path(&destination)?;
    }
    let _guard = acquire_file_mutation_locks([destination.clone()])?;
    if source.is_dir() {
        reject_recursive_directory_copy(&source, &destination)?;
        copy_dir_recursive(&source, &destination)?;
    } else if source.is_file() {
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)
                .map_err(|err| format!("Create parent dirs failed: {err}"))?;
        }
        fs::copy(&source, &destination).map_err(|err| format!("Copy file failed: {err}"))?;
    } else {
        return Err(format!("不是可复制的文件或文件夹: {from}"));
    }
    Ok(format!(
        "Copied {} to {}",
        workspace_display_path(workspace, &source),
        workspace_display_path(workspace, &destination)
    ))
}

pub fn glob_files(workspace: &NativeToolWorkspace, arguments: &Value) -> Result<String, String> {
    let pattern = required_string(arguments, "pattern")?;
    validate_glob_pattern(pattern)?;
    let include_hidden = arguments
        .get("include_hidden")
        .or_else(|| arguments.get("includeHidden"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let max_results = arguments
        .get("max_results")
        .or_else(|| arguments.get("maxResults"))
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .unwrap_or(200)
        .clamp(1, MAX_GLOB_RESULTS);
    let root = resolve_tool_read_path(
        workspace,
        arguments
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or("."),
    )?;
    if !root.is_dir() {
        return Err("glob_files path must be a directory".to_string());
    }

    let mut matches = Vec::new();
    for path in walk_paths(&root, true, include_hidden, MAX_SEARCH_FILES)? {
        let rel = relative_slash_path(&root, &path);
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        if glob_match(pattern, &rel) || (!pattern.contains('/') && glob_match(pattern, file_name)) {
            let metadata =
                fs::metadata(&path).map_err(|err| format!("Read metadata failed: {err}"))?;
            matches.push(path_info(workspace, &path, &metadata)?);
            if matches.len() >= max_results {
                break;
            }
        }
    }

    format_json(json!({
        "pattern": pattern,
        "matches": matches,
        "truncated": matches.len() >= max_results
    }))
}

pub fn search_files(workspace: &NativeToolWorkspace, arguments: &Value) -> Result<String, String> {
    let query = required_string(arguments, "query")?;
    let root = resolve_tool_read_path(
        workspace,
        arguments
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or("."),
    )?;
    if !root.is_dir() {
        return Err("search_files path must be a directory".to_string());
    }
    let case_sensitive = arguments
        .get("case_sensitive")
        .or_else(|| arguments.get("caseSensitive"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let include_hidden = arguments
        .get("include_hidden")
        .or_else(|| arguments.get("includeHidden"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let max_results = arguments
        .get("max_results")
        .or_else(|| arguments.get("maxResults"))
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .unwrap_or(100)
        .clamp(1, MAX_SEARCH_MATCHES);
    let needle = if case_sensitive {
        query.to_string()
    } else {
        query.to_lowercase()
    };

    let mut matches = Vec::new();
    for path in walk_paths(&root, true, include_hidden, MAX_SEARCH_FILES)? {
        if matches.len() >= max_results {
            break;
        }
        if !path.is_file() {
            continue;
        }
        let metadata = fs::metadata(&path).map_err(|err| format!("Read metadata failed: {err}"))?;
        if metadata.len() > MAX_SEARCH_FILE_BYTES {
            continue;
        }
        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };
        for (idx, line) in content.lines().enumerate() {
            let haystack = if case_sensitive {
                line.to_string()
            } else {
                line.to_lowercase()
            };
            if haystack.contains(&needle) {
                matches.push(json!({
                    "path": workspace_display_path(workspace, &path),
                    "line": idx + 1,
                    "text": line
                }));
                if matches.len() >= max_results {
                    break;
                }
            }
        }
    }

    format_json(json!({
        "query": query,
        "matches": matches,
        "truncated": matches.len() >= max_results
    }))
}

fn required_string<'a>(arguments: &'a Value, key: &str) -> Result<&'a str, String> {
    arguments
        .get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("{key} is required"))
}

fn format_json(value: Value) -> Result<String, String> {
    serde_json::to_string_pretty(&value)
        .map_err(|err| format!("Serialize tool result failed: {err}"))
}

fn validate_glob_pattern(pattern: &str) -> Result<(), String> {
    let pattern_path = Path::new(pattern);
    if pattern_path.is_absolute() {
        return Err(
            "glob_files pattern must be relative to the search path; put the directory in path instead."
                .to_string(),
        );
    }
    if pattern_path
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err("glob_files pattern cannot contain '..'.".to_string());
    }
    Ok(())
}

fn path_info(
    workspace: &NativeToolWorkspace,
    path: &Path,
    metadata: &fs::Metadata,
) -> Result<Value, String> {
    let kind = if metadata.is_dir() {
        "directory"
    } else if metadata.is_file() {
        "file"
    } else {
        "other"
    };
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs());
    Ok(json!({
        "path": workspace_display_path(workspace, path),
        "type": kind,
        "sizeBytes": metadata.len(),
        "modifiedAt": modified
    }))
}

fn is_hidden_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.starts_with('.'))
        .unwrap_or(false)
}

fn walk_paths(
    root: &Path,
    recursive: bool,
    include_hidden: bool,
    max_paths: usize,
) -> Result<Vec<PathBuf>, String> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).map_err(|err| format!("Read directory failed: {err}"))? {
            let entry = entry.map_err(|err| format!("Read directory entry failed: {err}"))?;
            let path = entry.path();
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("");
            if !include_hidden && name.starts_with('.') {
                continue;
            }
            let metadata = entry
                .metadata()
                .map_err(|err| format!("Read entry metadata failed: {err}"))?;
            if metadata.is_dir() {
                if recursive && !DEFAULT_IGNORED_DIRS.contains(&name) {
                    stack.push(path.clone());
                }
            }
            out.push(path);
            if out.len() >= max_paths {
                return Ok(out);
            }
        }
    }
    Ok(out)
}

fn relative_slash_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn glob_match(pattern: &str, value: &str) -> bool {
    let pattern_parts: Vec<&str> = pattern.split('/').filter(|part| !part.is_empty()).collect();
    let value_parts: Vec<&str> = value.split('/').filter(|part| !part.is_empty()).collect();
    glob_match_parts(&pattern_parts, &value_parts)
}

fn glob_match_parts(pattern: &[&str], value: &[&str]) -> bool {
    if pattern.is_empty() {
        return value.is_empty();
    }
    if pattern[0] == "**" {
        return glob_match_parts(&pattern[1..], value)
            || (!value.is_empty() && glob_match_parts(pattern, &value[1..]));
    }
    if value.is_empty() {
        return false;
    }
    segment_match(pattern[0], value[0]) && glob_match_parts(&pattern[1..], &value[1..])
}

fn segment_match(pattern: &str, value: &str) -> bool {
    let p = pattern.as_bytes();
    let v = value.as_bytes();
    let (mut pi, mut vi) = (0usize, 0usize);
    let mut star: Option<usize> = None;
    let mut star_match = 0usize;
    while vi < v.len() {
        if pi < p.len() && (p[pi] == b'?' || p[pi] == v[vi]) {
            pi += 1;
            vi += 1;
        } else if pi < p.len() && p[pi] == b'*' {
            star = Some(pi);
            star_match = vi;
            pi += 1;
        } else if let Some(star_idx) = star {
            pi = star_idx + 1;
            star_match += 1;
            vi = star_match;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == b'*' {
        pi += 1;
    }
    pi == p.len()
}

fn reject_workspace_root_delete(
    workspace: &NativeToolWorkspace,
    path: &Path,
) -> Result<(), String> {
    if let Some(project) = &workspace.project {
        if let Some(root) = project.root_path.as_ref() {
            if let Ok(root) = fs::canonicalize(root) {
                if path == root {
                    return Err("不能删除、移动或覆盖项目根目录。".to_string());
                }
            }
        }
    }
    Ok(())
}

fn reject_recursive_directory_copy(source: &Path, destination: &Path) -> Result<(), String> {
    if destination == source || destination.starts_with(source) {
        return Err("不能将文件夹复制到自身或自身的子目录。".to_string());
    }
    Ok(())
}

fn copy_dir_recursive(source: &Path, destination: &Path) -> Result<(), String> {
    fs::create_dir_all(destination)
        .map_err(|err| format!("Create destination dir failed: {err}"))?;
    for entry in fs::read_dir(source).map_err(|err| format!("Read source dir failed: {err}"))? {
        let entry = entry.map_err(|err| format!("Read source entry failed: {err}"))?;
        let from = entry.path();
        let to = destination.join(entry.file_name());
        let metadata = entry
            .metadata()
            .map_err(|err| format!("Read source metadata failed: {err}"))?;
        if metadata.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else if metadata.is_file() {
            fs::copy(&from, &to).map_err(|err| format!("Copy file failed: {err}"))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;

    #[test]
    fn read_file_allows_temp_paths() {
        let file = std::env::temp_dir().join(format!("kivio_read_{}.txt", uuid::Uuid::new_v4()));
        fs::write(&file, "alpha\nbeta\n").expect("write");

        let workspace = NativeToolWorkspace::global(&[]);
        let result =
            read_file(&workspace, &json!({ "path": file.to_string_lossy() })).expect("read");
        let canonical = fs::canonicalize(&file).expect("canonicalize temp file");
        assert_eq!(result.content, "alpha\nbeta\n");
        assert_eq!(result.path, canonical.to_string_lossy());
        assert_eq!(result.resolved_path, canonical.to_string_lossy());
        assert_eq!(result.total_lines, 2);
        assert_eq!(result.start_line, 1);
        assert_eq!(result.end_line, 2);
        assert!(!result.truncated);
        assert_eq!(result.next_offset, None);
        assert_eq!(result.read_state.scope, "full");

        let _ = fs::remove_file(file);
    }

    #[test]
    fn read_file_returns_range_metadata_for_partial_reads() {
        let file = std::env::temp_dir().join(format!("kivio_read_{}.txt", uuid::Uuid::new_v4()));
        fs::write(&file, "one\ntwo\nthree\nfour\n").expect("write");

        let workspace = NativeToolWorkspace::global(&[]);
        let result = read_file(
            &workspace,
            &json!({
                "path": file.to_string_lossy(),
                "offset": 2,
                "limit": 2
            }),
        )
        .expect("read range");

        assert_eq!(result.content, "two\nthree");
        assert_eq!(result.total_lines, 4);
        assert_eq!(result.start_line, 2);
        assert_eq!(result.end_line, 3);
        assert!(result.truncated);
        assert_eq!(result.next_offset, Some(4));
        assert_eq!(result.read_state.scope, "partial");

        let _ = fs::remove_file(file);
    }

    #[test]
    fn read_file_rejects_oversized_file_without_window_but_allows_offset_limit() {
        let file = std::env::temp_dir().join(format!("kivio_big_{}.txt", uuid::Uuid::new_v4()));
        let line = "x".repeat(1024);
        let mut body = String::new();
        for idx in 0..3000 {
            body.push_str(&format!("{idx} {line}\n"));
        }
        assert!(body.len() as u64 > MAX_READ_FILE_BYTES);
        fs::write(&file, &body).expect("write big file");

        let workspace = NativeToolWorkspace::global(&[]);
        let err = read_file(&workspace, &json!({ "path": file.to_string_lossy() })).unwrap_err();
        assert!(
            err.contains("offset/limit"),
            "error should hint at windowed reads: {err}"
        );

        let result = read_file(
            &workspace,
            &json!({
                "path": file.to_string_lossy(),
                "offset": 2001,
                "limit": 2
            }),
        )
        .expect("windowed read of oversized file");
        assert_eq!(result.total_lines, 3000);
        assert_eq!(result.start_line, 2001);
        assert_eq!(result.end_line, 2002);
        assert!(result.content.starts_with("2000 "));
        assert!(result.truncated);
        assert_eq!(result.next_offset, Some(2003));
        assert_eq!(result.read_state.scope, "partial");

        let _ = fs::remove_file(file);
    }

    #[test]
    fn write_file_allows_placeholder_phrases_in_new_and_prose_files() {
        let root = std::env::temp_dir().join(format!("kivio_prose_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).expect("mkdir");
        let workspace = NativeToolWorkspace::project(
            "proj_test".to_string(),
            "Test".to_string(),
            Some(root.to_string_lossy().into_owned()),
        );

        // New code file containing a placeholder phrase: allowed (nothing to lazily truncate).
        write_file(
            &workspace,
            &json!({ "path": "fresh.rs", "content": "// 省略 demo\nfn main() {}\n" }),
        )
        .expect("new code file with placeholder phrase");

        // Existing prose file: phrases like 省略 are normal text, allowed.
        write_file(&workspace, &json!({ "path": "notes.md", "content": "v1" })).expect("seed");
        write_file(
            &workspace,
            &json!({ "path": "notes.md", "content": "会议纪要：以下内容省略，详情保持不变。" }),
        )
        .expect("prose overwrite with placeholder phrase");

        // Existing code file: placeholder phrase means a lazy overwrite, rejected.
        let err = write_file(
            &workspace,
            &json!({ "path": "fresh.rs", "content": "fn main() {}\n// rest of file unchanged\n" }),
        )
        .unwrap_err();
        assert!(err.contains("placeholder"));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn edit_file_requires_unique_match_by_default() {
        let home = super::super::user_home_dir().expect("home");
        let dir = home.join(format!(".kivio_test_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("mkdir");
        let file = dir.join("sample.txt");
        fs::write(&file, "alpha\nbeta\nalpha\n").expect("write");

        let rel = file.to_string_lossy().to_string();
        let workspace = NativeToolWorkspace::global(&[]);
        let err = edit_file(
            &workspace,
            &json!({
                "path": rel,
                "old_string": "alpha",
                "new_string": "gamma"
            }),
        )
        .unwrap_err();
        assert!(err.contains("appears"));

        edit_file(
            &workspace,
            &json!({
                "path": rel,
                "old_string": "alpha",
                "new_string": "gamma",
                "replace_all": true
            }),
        )
        .expect("replace all");

        let content = fs::read_to_string(&file).expect("read");
        assert_eq!(content, "gamma\nbeta\ngamma\n");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn edit_file_reports_noop_when_old_equals_new() {
        let home = super::super::user_home_dir().expect("home");
        let dir = home.join(format!(".kivio_test_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("mkdir");
        let file = dir.join("sample.txt");
        fs::write(&file, "hello world").expect("write");

        let rel = file.to_string_lossy().to_string();
        let workspace = NativeToolWorkspace::global(&[]);
        let result = edit_file(
            &workspace,
            &json!({
                "path": rel,
                "old_string": "hello world",
                "new_string": "hello world"
            }),
        )
        .expect("noop edit");
        assert_eq!(result.files[0].operation, "noop");
        assert!(result
            .warnings
            .iter()
            .any(|warning| warning.contains("no changes made")));
        assert_eq!(fs::read_to_string(&file).expect("read"), "hello world");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn edit_file_matches_lf_old_string_against_crlf_file_and_keeps_crlf() {
        let root = std::env::temp_dir().join(format!("kivio_edit_crlf_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).expect("mkdir");
        let workspace = NativeToolWorkspace::project(
            "proj".to_string(),
            "T".to_string(),
            Some(root.to_string_lossy().into_owned()),
        );
        let file = root.join("crlf.txt");
        fs::write(&file, "line one\r\nline two\r\nline three\r\n").expect("write");

        // 模型给的是 LF old_string；文件是 CRLF —— 旧实现会 0 命中。
        let result = edit_file(
            &workspace,
            &json!({
                "path": "crlf.txt",
                "old_string": "line two\n",
                "new_string": "line 2\n",
            }),
        )
        .expect("LF old_string must match a CRLF file");
        assert!(result.ok);
        assert_eq!(result.files[0].operation, "edit");
        // diff 只反映真实变更，不把 CRLF→LF 算成整文件变更。
        assert_eq!(result.additions, 1);
        assert_eq!(result.removals, 1);
        // 写回保持 CRLF 风格。
        let on_disk = String::from_utf8(fs::read(&file).expect("read bytes")).expect("utf8");
        assert_eq!(on_disk, "line one\r\nline 2\r\nline three\r\n");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn edit_file_treats_crlf_vs_lf_only_change_as_noop() {
        let root = std::env::temp_dir().join(format!("kivio_edit_noop_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).expect("mkdir");
        let workspace = NativeToolWorkspace::project(
            "proj".to_string(),
            "T".to_string(),
            Some(root.to_string_lossy().into_owned()),
        );
        let file = root.join("file.txt");
        fs::write(&file, "alpha\r\nbeta\r\n").expect("write");

        // old/new 仅行尾写法不同，归一后相等 → 视为 noop，不改盘。
        let result = edit_file(
            &workspace,
            &json!({
                "path": "file.txt",
                "old_string": "alpha\r\nbeta",
                "new_string": "alpha\nbeta",
            }),
        )
        .expect("line-ending-only change is a noop");
        assert_eq!(result.files[0].operation, "noop");
        assert_eq!(
            String::from_utf8(fs::read(&file).expect("read")).expect("utf8"),
            "alpha\r\nbeta\r\n"
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn edit_file_lf_file_still_edits_and_keeps_lf() {
        let root = std::env::temp_dir().join(format!("kivio_edit_lf_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).expect("mkdir");
        let workspace = NativeToolWorkspace::project(
            "proj".to_string(),
            "T".to_string(),
            Some(root.to_string_lossy().into_owned()),
        );
        let file = root.join("lf.txt");
        fs::write(&file, "x\ny\nz\n").expect("write");

        let result = edit_file(
            &workspace,
            &json!({
                "path": "lf.txt",
                "old_string": "y\n",
                "new_string": "Y\n",
            }),
        )
        .expect("LF file edit");
        assert!(result.ok);
        let on_disk = String::from_utf8(fs::read(&file).expect("read")).expect("utf8");
        assert_eq!(on_disk, "x\nY\nz\n");
        assert!(!on_disk.contains('\r'), "LF file must not gain CR");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn write_file_returns_structured_diff_metadata() {
        let root = std::env::temp_dir().join(format!("kivio_write_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).expect("mkdir");
        let workspace = NativeToolWorkspace::project(
            "proj_test".to_string(),
            "Test".to_string(),
            Some(root.to_string_lossy().into_owned()),
        );

        let result = write_file(
            &workspace,
            &json!({
                "path": "hello.txt",
                "content": "alpha\nbeta\n"
            }),
        )
        .expect("write");

        assert_eq!(result.operation, "create");
        assert_eq!(result.resolved_path.as_deref(), Some("hello.txt"));
        assert_eq!(result.files[0].path, "hello.txt");
        assert_eq!(result.additions, 2);
        assert_eq!(result.removals, 0);
        assert!(result.diff.contains("+++ b/hello.txt"));
        assert!(result.diff.contains("+alpha"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn project_workspace_rejects_escape_paths() {
        let root = std::env::temp_dir().join(format!("kivio_project_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).expect("mkdir");
        let workspace = NativeToolWorkspace::project(
            "proj_test".to_string(),
            "Test".to_string(),
            Some(root.to_string_lossy().into_owned()),
        );

        let err = read_file(&workspace, &json!({ "path": "../secret.txt" })).unwrap_err();
        assert!(err.contains(".."));

        // Explicit absolute paths outside the project are allowed for reads,
        // matching non-project conversations.
        let outside = std::env::temp_dir().join(format!("kivio_outside_{}", uuid::Uuid::new_v4()));
        fs::write(&outside, "secret").expect("write outside");
        let result = read_file(&workspace, &json!({ "path": outside.to_string_lossy() }))
            .expect("absolute read outside project");
        assert_eq!(result.content, "secret");

        let _ = fs::remove_file(outside);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn project_workspace_allows_explicit_home_write_outside_root() {
        let root = std::env::temp_dir().join(format!("kivio_project_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).expect("mkdir");
        let workspace = NativeToolWorkspace::project(
            "proj_test".to_string(),
            "Test".to_string(),
            Some(root.to_string_lossy().into_owned()),
        );

        // Explicit home path escapes the project via global write rules.
        let home = super::super::user_home_dir().expect("home");
        let dir = home.join(format!(".kivio_escape_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("mkdir home target");
        let target = dir.join("note.html");
        write_file(
            &workspace,
            &json!({ "path": target.to_string_lossy(), "content": "<html></html>" }),
        )
        .expect("explicit absolute write outside project");
        assert_eq!(
            fs::read_to_string(&target).expect("read back"),
            "<html></html>"
        );

        // Relative paths still cannot escape, and blocked dirs stay blocked.
        let err = write_file(
            &workspace,
            &json!({ "path": "../escape.txt", "content": "x" }),
        )
        .unwrap_err();
        assert!(err.contains(".."));
        let blocked = home.join(".ssh/kivio_test_blocked.txt");
        let err = write_file(
            &workspace,
            &json!({ "path": blocked.to_string_lossy(), "content": "x" }),
        )
        .unwrap_err();
        assert!(err.contains(".ssh"));

        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn copy_path_rejects_directory_copy_into_self_or_child() {
        let root = std::env::temp_dir().join(format!("kivio_copy_{}", uuid::Uuid::new_v4()));
        let source = root.join("src");
        fs::create_dir_all(&source).expect("mkdir source");
        fs::write(source.join("file.txt"), "hello").expect("write source file");
        let workspace = NativeToolWorkspace::project(
            "proj_test".to_string(),
            "Test".to_string(),
            Some(root.to_string_lossy().into_owned()),
        );

        let same_err = copy_path(
            &workspace,
            &json!({
                "from": "src",
                "to": "src"
            }),
        )
        .unwrap_err();
        assert!(same_err.contains("自身"));

        let child_err = copy_path(
            &workspace,
            &json!({
                "from": "src",
                "to": "src/backup"
            }),
        )
        .unwrap_err();
        assert!(child_err.contains("自身"));
        assert!(!source.join("backup").exists());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn glob_files_rejects_path_like_patterns() {
        let root = std::env::temp_dir().join(format!("kivio_glob_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).expect("mkdir");
        fs::write(root.join("package.json"), "{}").expect("write package");
        let workspace = NativeToolWorkspace::project(
            "proj_test".to_string(),
            "Test".to_string(),
            Some(root.to_string_lossy().into_owned()),
        );

        let absolute_err = glob_files(
            &workspace,
            &json!({
                "pattern": format!("{}/*.json", root.display())
            }),
        )
        .unwrap_err();
        assert!(absolute_err.contains("relative"));

        let parent_err = glob_files(
            &workspace,
            &json!({
                "pattern": "../*.json"
            }),
        )
        .unwrap_err();
        assert!(parent_err.contains(".."));

        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn delete_path_removes_project_symlink_without_following_target() {
        let root = std::env::temp_dir().join(format!("kivio_link_root_{}", uuid::Uuid::new_v4()));
        let outside =
            std::env::temp_dir().join(format!("kivio_link_target_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).expect("mkdir root");
        fs::write(&outside, "outside").expect("write outside");
        let link = root.join("outside-link.txt");
        std::os::unix::fs::symlink(&outside, &link).expect("symlink");
        let workspace = NativeToolWorkspace::project(
            "proj_test".to_string(),
            "Test".to_string(),
            Some(root.to_string_lossy().into_owned()),
        );

        let result = delete_path(&workspace, &json!({ "path": "outside-link.txt" }))
            .expect("delete symlink");

        assert!(result.contains("outside-link.txt"));
        assert!(!link.exists());
        assert!(outside.exists());

        let _ = fs::remove_file(outside);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn write_file_overwrites_non_utf8_file_with_warning() {
        let root = std::env::temp_dir().join(format!("kivio_binary_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).expect("mkdir");
        fs::write(root.join("bin.dat"), [0xffu8, 0xfe, 0x01]).expect("write binary");
        let workspace = NativeToolWorkspace::project(
            "proj_test".to_string(),
            "Test".to_string(),
            Some(root.to_string_lossy().into_owned()),
        );

        let result = write_file(
            &workspace,
            &json!({
                "path": "bin.dat",
                "content": "clean\n"
            }),
        )
        .expect("overwrite non-utf8 file");

        assert_eq!(result.operation, "overwrite");
        assert!(result.diff.is_empty());
        assert_eq!(result.additions, 0);
        assert_eq!(result.removals, 0);
        assert!(result
            .warnings
            .iter()
            .any(|warning| warning.contains("not valid UTF-8")));
        assert_eq!(
            fs::read_to_string(root.join("bin.dat")).expect("read"),
            "clean\n"
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn edit_file_replace_all_reports_exact_stats_with_multiple_hunks() {
        let root = std::env::temp_dir().join(format!("kivio_hunks_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).expect("mkdir");
        let mut lines = vec!["needle old".to_string()];
        for i in 0..20 {
            lines.push(format!("unchanged line {i}"));
        }
        lines.push("needle old".to_string());
        let content = format!("{}\n", lines.join("\n"));
        fs::write(root.join("scatter.txt"), &content).expect("write scatter");
        let workspace = NativeToolWorkspace::project(
            "proj_test".to_string(),
            "Test".to_string(),
            Some(root.to_string_lossy().into_owned()),
        );

        let result = edit_file(
            &workspace,
            &json!({
                "path": "scatter.txt",
                "old_string": "old",
                "new_string": "new",
                "replace_all": true
            }),
        )
        .expect("replace all");

        assert_eq!(result.additions, 2);
        assert_eq!(result.removals, 2);
        assert_eq!(result.diff.matches("@@ -").count(), 2);
        let removed_lines = result
            .diff
            .lines()
            .filter(|line| line.starts_with('-') && !line.starts_with("---"))
            .count();
        let added_lines = result
            .diff
            .lines()
            .filter(|line| line.starts_with('+') && !line.starts_with("+++"))
            .count();
        assert_eq!(removed_lines, 2);
        assert_eq!(added_lines, 2);

        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn move_path_moves_project_symlink_without_following_target() {
        let root =
            std::env::temp_dir().join(format!("kivio_move_link_root_{}", uuid::Uuid::new_v4()));
        let outside =
            std::env::temp_dir().join(format!("kivio_move_link_target_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).expect("mkdir root");
        fs::write(&outside, "outside").expect("write outside");
        let link = root.join("outside-link.txt");
        std::os::unix::fs::symlink(&outside, &link).expect("symlink");
        let workspace = NativeToolWorkspace::project(
            "proj_test".to_string(),
            "Test".to_string(),
            Some(root.to_string_lossy().into_owned()),
        );

        let result = move_path(
            &workspace,
            &json!({
                "from": "outside-link.txt",
                "to": "moved-link.txt"
            }),
        )
        .expect("move symlink");

        assert!(result.contains("moved-link.txt"));
        assert!(!link.exists() && fs::symlink_metadata(&link).is_err());
        let moved = root.join("moved-link.txt");
        let metadata = fs::symlink_metadata(&moved).expect("moved metadata");
        assert!(metadata.file_type().is_symlink());
        assert_eq!(
            fs::read_to_string(&outside).expect("read outside"),
            "outside"
        );

        let _ = fs::remove_file(outside);
        let _ = fs::remove_dir_all(root);
    }
}
