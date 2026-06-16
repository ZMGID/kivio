//! Readable per-tool rendering for interactive tool cards (Phase 5c / 6a).
//!
//! Where 5b dumped a raw clipped JSON / preview blob under every tool card, this
//! module mirrors PI's `modes/interactive/components/tool-execution.ts`: each
//! tool's result is shaped into a compact, human-readable body —
//!
//! - `ls` / `find` / `glob_files`: a clean file/dir name list (basenames, dirs
//!   first), truncated with a `… +N more` line.
//! - `search_files` / `grep`: matched `file:line` references, truncated.
//! - `read_file`: a one-line `read <path> (N lines)` header from
//!   `structured_content`.
//! - `edit_file` / `write_file`: the unified diff rendered prominently with
//!   green `+` / red `-` line coloring, clipped.
//! - `bash` / `run_command`: the command echoed, then a tail of its output.
//!
//! **Structured-first parsing (6a).** Cards parse the tool's *structured* result
//! shape — `structured_content` when the tool carries it (read/edit/write),
//! else the JSON the listing/grep tools emit through their preview. They never
//! line-split a clipped JSON string (which is what produced raw `"entries":
//! [ … ] +N more` dumps for larger `ls`/`find`/`grep` results). The `+N more`
//! counts reflect the real entry/match count, not where a string was clipped.
//! If a result is genuinely unparseable (a blob truncated mid-JSON), a tolerant
//! `"path": "…"` scan recovers a clean name list; failing that, a single clipped
//! preview line — never a wall of JSON braces.
//!
//! Everything here is pure (`ToolCard` + width → `Vec<String>` of pre-colored
//! ANSI lines) so it is unit-testable without a TTY. The colors are emitted as
//! raw SGR escapes (matching the rest of the interactive module's `default_*`
//! themes) and stripped by the test helpers via `visible_width`.

use super::app::ToolCard;
use crate::chat::types::ToolCallStatus;
use crate::kivio_code::tui::components::{Text, Markdown, MarkdownTheme};
use crate::kivio_code::tui::render::Component;
use serde_json::Value;

/// Max list entries (ls/find) or match lines (grep) shown before a `+N more`.
const MAX_LIST_LINES: usize = 12;
/// Max diff lines shown in a card before clipping.
const MAX_DIFF_LINES: usize = 40;
/// Max output (tail) lines shown for bash.
const MAX_BASH_TAIL_LINES: usize = 16;
/// Hard cap on a single rendered detail line's source length before the Text
/// component word-wraps it (keeps very long lines from dominating the card).
const MAX_DETAIL_CHARS: usize = 4000;

const DIM: &str = "\x1b[2m";
const DIM_OFF: &str = "\x1b[22m";
const BOLD: &str = "\x1b[1m";
const BOLD_OFF: &str = "\x1b[22m";
const GREEN: &str = "\x1b[32m";
const RED: &str = "\x1b[31m";
const CYAN: &str = "\x1b[36m";
const COLOR_OFF: &str = "\x1b[39m";

/// The dim left gutter drawn before every body line, so a card's body reads as
/// one grouped unit visually separated from assistant text and the next card.
const GUTTER: &str = "\x1b[2m│\x1b[22m ";
/// Visible width the gutter occupies (`│` + space). Used to budget body width.
const GUTTER_WIDTH: u16 = 2;

/// The status glyph shown at the head of a card (shared with `app.rs`).
pub fn status_symbol(status: &ToolCallStatus) -> &'static str {
    match status {
        ToolCallStatus::Pending => "·",
        ToolCallStatus::Running => "▶",
        ToolCallStatus::Success => "✓",
        ToolCallStatus::Error => "✗",
        ToolCallStatus::Skipped => "⊘",
        ToolCallStatus::Cancelled => "⊗",
    }
}

/// The status glyph wrapped in its themed color: green success, red error, cyan
/// while running/pending, dim for skipped/cancelled. Keeps the header readable
/// at a glance without dumping raw SGR into the per-tool body code.
fn status_glyph_colored(status: &ToolCallStatus) -> String {
    let glyph = status_symbol(status);
    match status {
        ToolCallStatus::Success => format!("{GREEN}{glyph}{COLOR_OFF}"),
        ToolCallStatus::Error => format!("{RED}{glyph}{COLOR_OFF}"),
        ToolCallStatus::Running | ToolCallStatus::Pending => format!("{CYAN}{glyph}{COLOR_OFF}"),
        ToolCallStatus::Skipped | ToolCallStatus::Cancelled => format!("{DIM}{glyph}{DIM_OFF}"),
    }
}

/// A short, fixed-width text tag shown before the tool name in the header so the
/// card type is scannable at a glance. Plain ASCII (no emoji) keeps the column
/// alignment exact across terminals — emoji are double-width on some and
/// single on others, which would jitter the header.
fn tool_tag(name: &str) -> &'static str {
    match normalize_tool(name) {
        ToolKind::Read => "read",
        ToolKind::Listing => "list",
        ToolKind::Grep => "find",
        ToolKind::Mutation => "edit",
        ToolKind::Bash => "bash",
        ToolKind::Other => "tool",
    }
}

/// Render one tool card to ANSI lines for the given viewport width.
///
/// Layout is a grouped block:
///
/// ```text
///                          ← one blank line separating it from the previous block
/// ✓ list  src — path=.     ← header: colored glyph, fixed tag, bold name, dim summary
/// │ sub/                   ← body, indented under a dim left gutter
/// │ alpha.rs
/// ```
///
/// The leading blank line keeps adjacent cards (and surrounding assistant text)
/// from running together. The body is shaped per tool; errors short-circuit to a
/// red error block regardless of tool type. Every emitted line respects `width`.
pub fn render_tool_card(card: &ToolCard, width: u16) -> Vec<String> {
    let mut lines = Vec::new();
    // Blank separator line so cards don't visually merge with the preceding
    // block. A bare empty string renders as a blank terminal row.
    lines.push(String::new());
    lines.extend(header_lines(card, width));

    // An error result is shown the same way for every tool: a red, guttered block.
    if matches!(card.status, ToolCallStatus::Error) {
        if let Some(detail) = &card.detail {
            for raw in clip_lines(detail, MAX_BASH_TAIL_LINES) {
                lines.extend(body_line(&format!("{RED}{}{COLOR_OFF}", raw), width));
            }
        }
        return lines;
    }

    // Still running / pending: no body yet.
    if matches!(card.status, ToolCallStatus::Pending | ToolCallStatus::Running) {
        return lines;
    }

    lines.extend(body_lines(card, width));
    lines
}

/// The card header line(s): `<glyph> <tag> <name>  <summary>`.
///
/// The glyph is themed by status, the fixed-width tag makes the card type
/// scannable, the tool name is emphasized (bold), and the argument summary is
/// dimmed so it reads as metadata rather than chat text.
fn header_lines(card: &ToolCard, width: u16) -> Vec<String> {
    let glyph = status_glyph_colored(&card.status);
    let tag = tool_tag(&card.tool_name);
    let name = format!("{BOLD}{}{BOLD_OFF}", card.tool_name);
    let header = if card.summary.is_empty() {
        format!("{glyph} {DIM}{tag}{DIM_OFF} {name}")
    } else {
        format!(
            "{glyph} {DIM}{tag}{DIM_OFF} {name}  {DIM}{}{DIM_OFF}",
            card.summary
        )
    };
    text_line(&header, width)
}

/// The per-tool body. Dispatches on the (normalized) tool name; falls back to a
/// single collapsed preview line for anything unrecognized.
fn body_lines(card: &ToolCard, width: u16) -> Vec<String> {
    match normalize_tool(&card.tool_name) {
        ToolKind::Read => read_body(card, width),
        ToolKind::Listing => listing_body(card, width),
        ToolKind::Grep => grep_body(card, width),
        ToolKind::Mutation => mutation_body(card, width),
        ToolKind::Bash => bash_body(card, width),
        ToolKind::Other => preview_body(card, width),
    }
}

/// Tool families that share a rendering shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ToolKind {
    Read,
    Listing,
    Grep,
    Mutation,
    Bash,
    Other,
}

fn normalize_tool(name: &str) -> ToolKind {
    match name {
        "read" | "read_file" => ToolKind::Read,
        "ls" | "list_dir" | "find" | "glob_files" => ToolKind::Listing,
        "grep" | "search_files" => ToolKind::Grep,
        "write" | "write_file" | "edit" | "edit_file" => ToolKind::Mutation,
        "bash" | "run_command" => ToolKind::Bash,
        _ => ToolKind::Other,
    }
}

/// `read_file`: a single dim header `read <path> (N lines)`, preferring the
/// structured content for an accurate range; falls back to the summary path.
fn read_body(card: &ToolCard, width: u16) -> Vec<String> {
    let header = read_header(card);
    body_line(&format!("{DIM}{header}{DIM_OFF}"), width)
}

fn read_header(card: &ToolCard) -> String {
    if let Some(sc) = &card.structured_content {
        let path = sc.get("path").and_then(|v| v.as_str()).unwrap_or("");
        let total = sc.get("total_lines").and_then(|v| v.as_u64());
        let start = sc.get("start_line").and_then(|v| v.as_u64());
        let end = sc.get("end_line").and_then(|v| v.as_u64());
        if !path.is_empty() {
            return match (total, start, end) {
                (Some(total), Some(start), Some(end)) if start > 1 || end < total => {
                    format!("read {path} (lines {start}-{end} of {total})")
                }
                (Some(total), _, _) => format!("read {path} ({total} lines)"),
                _ => format!("read {path}"),
            };
        }
    }
    // Fallback: derive from the summary (`path=...`).
    match arg_after_eq(&card.summary, "path") {
        Some(path) => format!("read {path}"),
        None => "read".to_string(),
    }
}

/// The structured result to render a card from, **structured-first**: prefer
/// `structured_content` (the complete, parseable result the tool produced);
/// otherwise try to parse the (model-facing, possibly truncated) `detail`
/// preview as whole JSON. Returns `None` when neither is available/parseable —
/// callers then fall back to a string scan or a clipped preview.
///
/// This is the crux of the tool-card fix: native listing/grep tools
/// (`list_dir`/`glob_files`/`search_files`) go through the SyncText registry
/// path, which leaves `structured_content = None` and stuffs the full JSON into
/// the preview — so for those we parse `detail`. read/edit/write carry real
/// `structured_content`, so they use it directly.
fn card_payload(card: &ToolCard) -> Option<Value> {
    if let Some(sc) = &card.structured_content {
        return Some(sc.clone());
    }
    let detail = card.detail.as_deref()?.trim();
    // `bash` etc. sometimes prefix with `stdout:`; only attempt a clean JSON
    // parse here — partial/truncated blobs intentionally fail and fall through.
    serde_json::from_str::<Value>(detail).ok()
}

/// `ls` / `find` / `glob_files`: a clean per-entry name list, grouped into
/// directories and files, clipped with a `… +N more` line.
///
/// These tools (`list_dir`/`glob_files`/`find`) emit a structured JSON object
/// with an `entries`/`matches` array of `{ path, type, sizeBytes, modifiedAt }`.
/// We parse that structure (structured-first; see [`card_payload`]) and show
/// each entry's **basename**, dirs first. The `+N more` count reflects the real
/// number of entries — never an artifact of where a truncated JSON string was
/// clipped, which is what produced the raw `"entries": [ … ] +N more` dumps.
fn listing_body(card: &ToolCard, width: u16) -> Vec<String> {
    let names = card_payload(card)
        .as_ref()
        .and_then(listing_names)
        .or_else(|| card.detail.as_deref().and_then(listing_names_from_text));

    let Some(names) = names else {
        // No structured array and no parseable preview: clip the raw preview to
        // a single line rather than dumping a wall of JSON braces.
        return clipped_preview_line(card, width);
    };
    if names.is_empty() {
        return body_line(&format!("{DIM}(empty){DIM_OFF}"), width);
    }

    // Partition while preserving order: directories first, then files/other.
    let mut dirs: Vec<String> = Vec::new();
    let mut files: Vec<String> = Vec::new();
    for entry in &names {
        if entry.is_dir {
            dirs.push(format!("{}/", entry.name));
        } else {
            files.push(entry.name.clone());
        }
    }
    let ordered: Vec<String> = dirs.into_iter().chain(files).collect();
    let total = ordered.len();
    let shown = total.min(MAX_LIST_LINES);

    let mut lines = Vec::new();
    for name in &ordered[..shown] {
        lines.extend(body_line(name, width));
    }
    if total > shown {
        let more = total - shown;
        lines.extend(body_line(&format!("{DIM}… +{more} more{DIM_OFF}"), width));
    }
    lines
}

/// `grep` / `search_files`: matched `file:line` references, clipped with a
/// `… +N more` line.
///
/// `search_files` emits a structured JSON object whose shape depends on
/// `output_mode`: `matches` (`{ path, line, text }`) for content mode, `files`
/// (a string array) for `files_with_matches`, or `counts` (`{ path, count }`)
/// for `count`. We parse the structure (structured-first), formatting each as a
/// concise `path:line` (or `path` / `path (N)`) reference. The `+N more` count
/// is the real match count, not a truncation artifact.
fn grep_body(card: &ToolCard, width: u16) -> Vec<String> {
    let refs = card_payload(card)
        .as_ref()
        .and_then(grep_refs)
        .or_else(|| card.detail.as_deref().and_then(grep_refs_from_text));

    let Some(refs) = refs else {
        return clipped_preview_line(card, width);
    };
    if refs.is_empty() {
        return body_line(&format!("{DIM}no matches{DIM_OFF}"), width);
    }

    let total = refs.len();
    let shown = total.min(MAX_LIST_LINES);
    let mut lines = Vec::new();
    for m in &refs[..shown] {
        // Dim so the `path:line` reference reads as a reference, not chat text.
        lines.extend(body_line(&format!("{DIM}{m}{DIM_OFF}"), width));
    }
    if total > shown {
        let more = total - shown;
        lines.extend(body_line(&format!("{DIM}… +{more} more{DIM_OFF}"), width));
    }
    lines
}

/// One listing entry: its display basename and whether it's a directory.
struct ListEntry {
    name: String,
    is_dir: bool,
}

/// Extract listing entries from a parsed tool result. Handles `list_dir`
/// (`entries`), `glob_files`/`find` (`matches`), each an array of
/// `{ path, type }` objects, plus a bare string array as a defensive fallback.
fn listing_names(value: &Value) -> Option<Vec<ListEntry>> {
    let array = value
        .get("entries")
        .or_else(|| value.get("matches"))
        .or_else(|| value.get("files"))
        .and_then(Value::as_array)?;
    let entries = array
        .iter()
        .filter_map(|item| match item {
            Value::Object(_) => {
                let path = item.get("path").and_then(Value::as_str)?;
                let is_dir = item
                    .get("type")
                    .and_then(Value::as_str)
                    .map(|t| t == "dir" || t == "directory")
                    .unwrap_or(false);
                Some(ListEntry { name: basename(path), is_dir })
            }
            // `files_with_matches` (or a plain glob list) is an array of strings.
            Value::String(path) => Some(ListEntry { name: basename(path), is_dir: false }),
            _ => None,
        })
        .collect();
    Some(entries)
}

/// Extract grep references from a parsed `search_files` result. Supports all
/// three output modes; returns `path:line` / `path` / `path (N)` strings.
fn grep_refs(value: &Value) -> Option<Vec<String>> {
    if let Some(matches) = value.get("matches").and_then(Value::as_array) {
        let refs = matches
            .iter()
            .filter_map(|m| {
                let path = m.get("path").and_then(Value::as_str)?;
                match m.get("line").and_then(Value::as_u64) {
                    Some(line) => Some(format!("{path}:{line}")),
                    None => Some(path.to_string()),
                }
            })
            .collect();
        return Some(refs);
    }
    if let Some(files) = value.get("files").and_then(Value::as_array) {
        let refs = files
            .iter()
            .filter_map(|f| f.as_str().map(str::to_string))
            .collect();
        return Some(refs);
    }
    if let Some(counts) = value.get("counts").and_then(Value::as_array) {
        let refs = counts
            .iter()
            .filter_map(|c| {
                let path = c.get("path").and_then(Value::as_str)?;
                let count = c.get("count").and_then(Value::as_u64).unwrap_or(0);
                Some(format!("{path} ({count})"))
            })
            .collect();
        return Some(refs);
    }
    None
}

/// Last path segment (forward- or back-slash) of a display path.
fn basename(path: &str) -> String {
    path.rsplit(|c| c == '/' || c == '\\')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(path)
        .to_string()
}

/// Fallback path-extraction from a (possibly truncated) preview string: pulls
/// every `"path": "<value>"` occurrence out by scanning, so even a JSON blob
/// clipped mid-object yields a clean name list instead of raw braces. Returns
/// `None` when the text has no recognizable structure at all (so the caller can
/// fall through to a clipped preview).
fn listing_names_from_text(text: &str) -> Option<Vec<ListEntry>> {
    let paths = extract_json_string_values(text, "path");
    if paths.is_empty() {
        return None;
    }
    Some(
        paths
            .into_iter()
            .map(|p| ListEntry { name: basename(&p), is_dir: false })
            .collect(),
    )
}

/// Fallback grep-reference extraction from a (possibly truncated) preview: pulls
/// `"path"` values out by scanning. Line numbers aren't reliably recoverable
/// from a clipped blob, so this yields bare paths. `None` when no structure.
fn grep_refs_from_text(text: &str) -> Option<Vec<String>> {
    let paths = extract_json_string_values(text, "path");
    if paths.is_empty() {
        return None;
    }
    Some(paths)
}

/// Scan `text` for every `"<key>": "<value>"` occurrence and collect the string
/// values, JSON-unescaping `\"`, `\\`, `\n`, `\t`, `\/`. Tolerant of a blob
/// truncated mid-value (the trailing partial is dropped). Used only as a last
/// resort when the result couldn't be parsed as whole JSON.
fn extract_json_string_values(text: &str, key: &str) -> Vec<String> {
    let needle = format!("\"{key}\"");
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(pos) = rest.find(&needle) {
        rest = &rest[pos + needle.len()..];
        // Expect `:` then an opening quote (allow whitespace between).
        let after_colon = match rest.find(':') {
            Some(c) => &rest[c + 1..],
            None => break,
        };
        let trimmed = after_colon.trim_start();
        let Some(body) = trimmed.strip_prefix('"') else {
            rest = after_colon;
            continue;
        };
        // Read until the closing unescaped quote.
        let mut value = String::new();
        let mut chars = body.char_indices();
        let mut closed = false;
        let mut consumed = 0usize;
        while let Some((idx, c)) = chars.next() {
            consumed = idx + c.len_utf8();
            match c {
                '\\' => {
                    if let Some((nidx, esc)) = chars.next() {
                        consumed = nidx + esc.len_utf8();
                        match esc {
                            'n' => value.push('\n'),
                            't' => value.push('\t'),
                            'r' => value.push('\r'),
                            '"' => value.push('"'),
                            '\\' => value.push('\\'),
                            '/' => value.push('/'),
                            other => value.push(other),
                        }
                    }
                }
                '"' => {
                    closed = true;
                    break;
                }
                _ => value.push(c),
            }
        }
        rest = &body[consumed..];
        if closed && !value.is_empty() {
            out.push(value);
        }
    }
    out
}

/// A clean single clipped preview line (newlines → spaces), used when a listing
/// or grep result is neither structured nor parseable — never a JSON-brace wall.
fn clipped_preview_line(card: &ToolCard, width: u16) -> Vec<String> {
    let Some(detail) = card.detail.as_deref() else {
        return body_line(&format!("{DIM}(no result){DIM_OFF}"), width);
    };
    let collapsed = detail.replace('\n', " ");
    let collapsed = clip_chars(collapsed.trim(), 200);
    body_line(&format!("{DIM}{collapsed}{DIM_OFF}"), width)
}

/// `write_file` / `edit_file`: the unified diff, green `+` / red `-`, clipped.
fn mutation_body(card: &ToolCard, width: u16) -> Vec<String> {
    let Some(diff) = mutation_diff(card) else {
        // No diff (e.g. no-op write): fall back to the model-facing summary.
        return preview_body(card, width);
    };
    diff_lines(&diff, width)
}

/// Extract the unified diff for a mutation card: prefer `structured_content.diff`
/// (full diff); fall back to the precomputed `card.diff`.
fn mutation_diff(card: &ToolCard) -> Option<String> {
    if let Some(sc) = &card.structured_content {
        if let Some(diff) = sc.get("diff").and_then(|d| d.as_str()) {
            if !diff.trim().is_empty() {
                return Some(diff.to_string());
            }
        }
    }
    card.diff.clone().filter(|d| !d.trim().is_empty())
}

/// Color a unified diff: `+` green, `-` red, `@@` cyan, everything else dim.
/// Clipped to [`MAX_DIFF_LINES`] with a trailing `… diff clipped` note. Each
/// line is rendered under the card gutter so the diff sits inside the card.
fn diff_lines(diff: &str, width: u16) -> Vec<String> {
    let raw: Vec<&str> = diff.lines().collect();
    let total = raw.len();
    let shown = total.min(MAX_DIFF_LINES);
    let mut lines = Vec::new();
    for line in &raw[..shown] {
        let colored = color_diff_line(line);
        lines.extend(body_line(&colored, width));
    }
    if total > shown {
        let more = total - shown;
        lines.extend(body_line(
            &format!("{DIM}… {more} more diff line(s) ({shown} of {total} shown){DIM_OFF}"),
            width,
        ));
    }
    lines
}

fn color_diff_line(line: &str) -> String {
    // `+++`/`---` file headers are part of the hunk frame; dim them so the
    // actual `+`/`-` content lines stand out.
    if line.starts_with("+++") || line.starts_with("---") {
        format!("{DIM}{line}{DIM_OFF}")
    } else if line.starts_with('+') {
        format!("{GREEN}{line}{COLOR_OFF}")
    } else if line.starts_with('-') {
        format!("{RED}{line}{COLOR_OFF}")
    } else if line.starts_with("@@") {
        format!("{CYAN}{line}{COLOR_OFF}")
    } else {
        format!("{DIM}{line}{DIM_OFF}")
    }
}

/// `bash` / `run_command`: the command (from the summary) then a clipped tail of
/// its output. The tool's text result is the command output.
fn bash_body(card: &ToolCard, width: u16) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(cmd) = arg_after_eq(&card.summary, "command") {
        // The command line, prefixed with a cyan `$` prompt so it reads as the
        // invocation rather than output.
        lines.extend(body_line(&format!("{CYAN}${COLOR_OFF} {BOLD}{cmd}{BOLD_OFF}"), width));
    }
    if let Some(detail) = &card.detail {
        let output: Vec<&str> = detail.lines().collect();
        let total = output.len();
        // Keep the *tail* (most shells' useful output is at the end).
        let start = total.saturating_sub(MAX_BASH_TAIL_LINES);
        if start > 0 {
            lines.extend(body_line(
                &format!("{DIM}… {start} earlier line(s) hidden{DIM_OFF}"),
                width,
            ));
        }
        for line in &output[start..] {
            lines.extend(body_line(&format!("{DIM}{line}{DIM_OFF}"), width));
        }
    }
    lines
}

/// Fallback: a single collapsed preview line (newlines → spaces), clipped. Used
/// for `web_fetch` and any unrecognized tool. Rendered through Markdown (so
/// fenced/inline formatting reads cleanly) at the gutter-reduced width, then
/// each produced line is placed under the card gutter.
fn preview_body(card: &ToolCard, width: u16) -> Vec<String> {
    let Some(detail) = &card.detail else {
        return Vec::new();
    };
    let collapsed = detail.replace('\n', " ");
    let collapsed = clip_chars(&collapsed, MAX_DETAIL_CHARS);
    let inner_width = width.saturating_sub(GUTTER_WIDTH).max(1);
    let mut md = Markdown::new(collapsed, 0, 0, MarkdownTheme::plain(), None);
    md.render(inner_width)
        .into_iter()
        .map(|line| format!("{GUTTER}{line}"))
        .collect()
}

// ---- helpers ----

/// Render one source line through the `Text` component (word-wrap + width clamp).
fn text_line(s: &str, width: u16) -> Vec<String> {
    let mut t = Text::new(s.to_string(), 1, 0, None);
    t.render(width)
}

/// Render one body line under the dim left gutter (`│ `), word-wrapping the
/// content to the gutter-reduced width so wrapped continuations stay aligned
/// under the same gutter and nothing exceeds `width`.
///
/// `content` may carry its own SGR color (diff +/-, dim refs, the bash `$`);
/// the gutter is prepended after wrapping so the bar color never bleeds into it.
fn body_line(content: &str, width: u16) -> Vec<String> {
    let inner_width = width.saturating_sub(GUTTER_WIDTH).max(1);
    let mut t = Text::new(content.to_string(), 0, 0, None);
    t.render(inner_width)
        .into_iter()
        .map(|line| format!("{GUTTER}{line}"))
        .collect()
}

/// Split `text` into at most `max` non-empty lines.
fn clip_lines(text: &str, max: usize) -> Vec<String> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .take(max)
        .map(|l| l.to_string())
        .collect()
}

/// Char-safe truncate to `max` chars, appending `…` when clipped.
fn clip_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        let head: String = text.chars().take(max).collect();
        format!("{head}…")
    }
}

/// Pull the value after `key=` out of a `key=value` summary (the summary the
/// card builder produced from the call arguments). Returns the remainder of the
/// summary after the first `key=`.
fn arg_after_eq(summary: &str, key: &str) -> Option<String> {
    let needle = format!("{key}=");
    summary
        .strip_prefix(&needle)
        .map(|rest| rest.to_string())
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kivio_code::tui::text_width::visible_width;

    fn card(name: &str, status: ToolCallStatus) -> ToolCard {
        ToolCard {
            id: "c1".to_string(),
            tool_name: name.to_string(),
            status,
            summary: String::new(),
            detail: None,
            diff: None,
            structured_content: None,
        }
    }

    // ---- real-output card builders ----
    //
    // These run the *actual* native tools against a throwaway temp dir and feed
    // the real result into the card the same way the registry/executor does, so
    // the card tests assert against the shapes the tools genuinely emit (not
    // hand-written JSON). `list_dir`/`glob_files`/`search_files` are SyncText
    // tools: the executor leaves `structured_content = None` and puts the JSON
    // string into the preview (→ `card.detail`). `read_file` carries real
    // `structured_content`. We mirror both.

    use crate::chat::types::ToolCallRecord;
    use crate::native_tools::NativeToolWorkspace;

    /// A unique scratch project dir for a single test, returned with a workspace
    /// scoped to it.
    fn scratch() -> (std::path::PathBuf, NativeToolWorkspace) {
        let dir =
            std::env::temp_dir().join(format!("kivio_card_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("mkdir scratch");
        let ws = NativeToolWorkspace::project(
            "card".into(),
            "Card".into(),
            Some(dir.to_string_lossy().into_owned()),
        );
        (dir, ws)
    }

    /// Build a `ToolCard` for a SyncText tool (list_dir/glob_files/search_files):
    /// JSON output goes into the preview/detail, `structured_content = None`.
    fn text_tool_card(name: &str, summary: &str, json_output: String) -> ToolCard {
        let mut record = base_record(name, summary);
        record.result_preview = Some(json_output);
        ToolCard::from_record(&record)
    }

    fn base_record(name: &str, summary: &str) -> ToolCallRecord {
        ToolCallRecord {
            id: "r1".to_string(),
            name: name.to_string(),
            source: "native".to_string(),
            server_id: None,
            arguments: summary.to_string(),
            status: ToolCallStatus::Success,
            result_preview: None,
            error: None,
            duration_ms: None,
            started_at: None,
            completed_at: None,
            round: 0,
            sensitive: false,
            artifacts: Vec::new(),
            trace_id: None,
            span_id: None,
            structured_content: None,
        }
    }

    fn joined(card: &ToolCard, width: u16) -> String {
        render_tool_card(card, width).join("\n")
    }

    fn strip_ansi(s: &str) -> String {
        // crude SGR stripper for assertions
        let mut out = String::new();
        let mut chars = s.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\x1b' {
                while let Some(&n) = chars.peek() {
                    chars.next();
                    if n == 'm' {
                        break;
                    }
                }
            } else {
                out.push(c);
            }
        }
        out
    }

    #[test]
    fn header_shows_glyph_tool_and_summary() {
        let mut c = card("read", ToolCallStatus::Success);
        c.summary = "path=src/main.rs".to_string();
        let text = strip_ansi(&joined(&c, 60));
        assert!(text.contains("read"));
        assert!(text.contains("src/main.rs"));
    }

    #[test]
    fn read_body_uses_real_read_file_structured_content() {
        // Drive the real read_file tool, then build the card from its real
        // structured_content (read_file IS a SyncResult tool that carries it).
        let (dir, ws) = scratch();
        let file = dir.join("lib.rs");
        let body: String = (0..120).map(|i| format!("line {i}\n")).collect();
        std::fs::write(&file, body).expect("write file");
        let result = crate::native_tools::read_file(
            &ws,
            &serde_json::json!({ "path": file.to_string_lossy() }),
        )
        .expect("read_file");
        let mut record = base_record("read_file", "path=lib.rs");
        record.structured_content =
            Some(serde_json::to_value(&result).expect("serialize read result"));
        record.result_preview = Some("ignored — structured wins".to_string());
        let c = ToolCard::from_record(&record);

        let text = strip_ansi(&joined(&c, 80));
        assert!(text.contains("lib.rs (120 lines)"), "{text}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn read_body_shows_range_when_windowed() {
        let mut c = card("read_file", ToolCallStatus::Success);
        c.structured_content = Some(serde_json::json!({
            "path": "big.txt", "total_lines": 500, "start_line": 10, "end_line": 60,
        }));
        let text = strip_ansi(&joined(&c, 80));
        assert!(text.contains("lines 10-60 of 500"), "{text}");
    }

    #[test]
    fn listing_body_lists_real_list_dir_entries_with_dirs_first() {
        // Real list_dir output → its JSON (with `entries: [{path,type,...}]`)
        // becomes the SyncText preview, structured_content stays None.
        let (dir, ws) = scratch();
        std::fs::create_dir_all(dir.join("sub")).expect("mkdir sub");
        std::fs::write(dir.join("alpha.rs"), "a").expect("write alpha");
        std::fs::write(dir.join("beta.rs"), "b").expect("write beta");
        let out = crate::native_tools::list_dir(
            &ws,
            &serde_json::json!({ "path": dir.to_string_lossy() }),
        )
        .expect("list_dir");
        let c = text_tool_card("ls", "path=.", out);

        let text = strip_ansi(&joined(&c, 80));
        // Clean basenames, NOT raw JSON keys.
        assert!(text.contains("sub/"), "dir should render with trailing slash: {text}");
        assert!(text.contains("alpha.rs"), "{text}");
        assert!(text.contains("beta.rs"), "{text}");
        assert!(!text.contains("\"entries\""), "must not dump raw JSON: {text}");
        assert!(!text.contains("sizeBytes"), "must not dump raw JSON: {text}");
        assert!(!text.contains("modifiedAt"), "must not dump raw JSON: {text}");
        // Directory listed before files.
        let dir_pos = text.find("sub/").unwrap();
        let file_pos = text.find("alpha.rs").unwrap();
        assert!(dir_pos < file_pos, "dirs should sort before files: {text}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn listing_body_truncates_large_real_listing_with_correct_count() {
        // 30 real files → 12 shown, "+18 more" — and the count is the REAL entry
        // count, not an artifact of where a JSON string was clipped.
        let (dir, ws) = scratch();
        for i in 0..30 {
            std::fs::write(dir.join(format!("f{i:02}.rs")), "x").expect("write");
        }
        let out = crate::native_tools::list_dir(
            &ws,
            &serde_json::json!({ "path": dir.to_string_lossy() }),
        )
        .expect("list_dir");
        let c = text_tool_card("ls", "path=.", out);

        let text = strip_ansi(&joined(&c, 80));
        assert!(text.contains("f00.rs"), "{text}");
        assert!(text.contains("+18 more"), "expected real count +18 more: {text}");
        assert!(!text.contains("\"entries\""), "must not dump raw JSON: {text}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn listing_body_lists_real_glob_matches() {
        let (dir, ws) = scratch();
        std::fs::create_dir_all(dir.join("src")).expect("mkdir src");
        std::fs::write(dir.join("src/main.rs"), "fn main(){}").expect("write main");
        std::fs::write(dir.join("src/lib.rs"), "pub fn x(){}").expect("write lib");
        std::fs::write(dir.join("README.md"), "# hi").expect("write readme");
        let out = crate::native_tools::glob_files(
            &ws,
            &serde_json::json!({ "pattern": "**/*.rs", "path": dir.to_string_lossy() }),
        )
        .expect("glob_files");
        let c = text_tool_card("find", "pattern=**/*.rs", out);

        let text = strip_ansi(&joined(&c, 80));
        assert!(text.contains("main.rs"), "{text}");
        assert!(text.contains("lib.rs"), "{text}");
        assert!(!text.contains("README"), "glob filtered to *.rs: {text}");
        assert!(!text.contains("\"matches\""), "must not dump raw JSON: {text}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn listing_empty_real_dir_shows_empty_marker() {
        let (dir, ws) = scratch();
        let out = crate::native_tools::list_dir(
            &ws,
            &serde_json::json!({ "path": dir.to_string_lossy() }),
        )
        .expect("list_dir");
        let c = text_tool_card("ls", "path=.", out);
        let text = strip_ansi(&joined(&c, 80));
        assert!(text.contains("(empty)"), "{text}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn grep_body_shows_real_search_files_matches_as_file_line() {
        let (dir, ws) = scratch();
        std::fs::create_dir_all(dir.join("src")).expect("mkdir src");
        std::fs::write(
            dir.join("src/a.rs"),
            "let x = 1;\nlet TODO = here;\nmore\nTODO again\n",
        )
        .expect("write a");
        let out = crate::native_tools::search_files(
            &ws,
            &serde_json::json!({ "query": "TODO", "path": dir.to_string_lossy() }),
        )
        .expect("search_files");
        let c = text_tool_card("grep", "query=TODO", out);

        let text = strip_ansi(&joined(&c, 100));
        // file:line references, not raw JSON braces.
        assert!(text.contains("a.rs:2"), "expected file:line ref: {text}");
        assert!(text.contains("a.rs:4"), "expected file:line ref: {text}");
        assert!(!text.contains("\"matches\""), "must not dump raw JSON: {text}");
        assert!(!text.contains("\"text\""), "must not dump raw JSON: {text}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn grep_body_truncates_large_real_match_set_with_correct_count() {
        let (dir, ws) = scratch();
        // One file, 20 matching lines → 20 matches, 12 shown, "+8 more".
        let body: String = (0..20).map(|_| "TODO line\n").collect();
        std::fs::write(dir.join("big.rs"), body).expect("write big");
        let out = crate::native_tools::search_files(
            &ws,
            &serde_json::json!({ "query": "TODO", "path": dir.to_string_lossy() }),
        )
        .expect("search_files");
        let c = text_tool_card("grep", "query=TODO", out);

        let text = strip_ansi(&joined(&c, 100));
        assert!(text.contains("big.rs:1"), "{text}");
        assert!(text.contains("+8 more"), "expected real count +8 more: {text}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn grep_no_matches_real_empty_result() {
        let (dir, ws) = scratch();
        std::fs::write(dir.join("a.rs"), "nothing here\n").expect("write a");
        let out = crate::native_tools::search_files(
            &ws,
            &serde_json::json!({ "query": "ZZZ_NOPE", "path": dir.to_string_lossy() }),
        )
        .expect("search_files");
        let c = text_tool_card("search_files", "query=ZZZ_NOPE", out);
        let text = strip_ansi(&joined(&c, 80));
        assert!(text.contains("no matches"), "{text}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn listing_recovers_from_truncated_json_blob_without_dumping_braces() {
        // Simulate a preview clipped mid-JSON (what `max_tool_output_chars`
        // truncation produces): whole-JSON parse fails, so the card falls back
        // to scanning `"path"` values — still a clean name list, no braces.
        let (dir, ws) = scratch();
        for i in 0..50 {
            std::fs::write(dir.join(format!("file{i:02}.rs")), "x").expect("write");
        }
        let full = crate::native_tools::list_dir(
            &ws,
            &serde_json::json!({ "path": dir.to_string_lossy() }),
        )
        .expect("list_dir");
        // Clip mid-object so serde_json::from_str fails.
        let clipped: String = full.chars().take(full.chars().count() / 3).collect();
        assert!(
            serde_json::from_str::<serde_json::Value>(&clipped).is_err(),
            "test precondition: clipped blob must be unparseable"
        );
        let c = text_tool_card("ls", "path=.", clipped);

        let text = strip_ansi(&joined(&c, 80));
        assert!(text.contains("file00.rs"), "recovered a name: {text}");
        assert!(!text.contains("\"path\""), "must not dump raw JSON keys: {text}");
        assert!(!text.contains("sizeBytes"), "must not dump raw JSON: {text}");
        assert!(!text.contains('{'), "must not dump JSON braces: {text}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn mutation_body_renders_colored_diff() {
        let mut c = card("edit_file", ToolCallStatus::Success);
        c.structured_content = Some(serde_json::json!({
            "diff": "@@ -1 +1 @@\n-old line\n+new line",
        }));
        let raw = joined(&c, 80);
        // raw contains the green/red SGR codes
        assert!(raw.contains(GREEN), "expected green for additions");
        assert!(raw.contains(RED), "expected red for removals");
        let text = strip_ansi(&raw);
        assert!(text.contains("+new line"));
        assert!(text.contains("-old line"));
    }

    #[test]
    fn mutation_body_clips_long_diff() {
        let mut c = card("write_file", ToolCallStatus::Success);
        let big: Vec<String> = (0..100).map(|i| format!("+line {i}")).collect();
        c.structured_content = Some(serde_json::json!({ "diff": big.join("\n") }));
        let text = strip_ansi(&joined(&c, 80));
        assert!(text.contains("more diff line(s)"), "{text}");
        assert!(text.contains("40 of 100 shown"), "{text}");
    }

    #[test]
    fn mutation_falls_back_to_preview_without_diff() {
        let mut c = card("write_file", ToolCallStatus::Success);
        c.detail = Some("wrote new.txt (+0 -0)".to_string());
        let text = strip_ansi(&joined(&c, 80));
        assert!(text.contains("wrote new.txt"));
    }

    #[test]
    fn bash_body_shows_command_and_output_tail() {
        let mut c = card("bash", ToolCallStatus::Success);
        c.summary = "command=cargo test".to_string();
        let out: Vec<String> = (0..30).map(|i| format!("line {i}")).collect();
        c.detail = Some(out.join("\n"));
        let text = strip_ansi(&joined(&c, 80));
        assert!(text.contains("cargo test"));
        // tail kept: last line present, earliest hidden
        assert!(text.contains("line 29"));
        assert!(text.contains("earlier line(s) hidden"), "{text}");
        assert!(!text.contains("line 0\n") || text.contains("hidden"));
    }

    #[test]
    fn error_status_shows_red_error_regardless_of_tool() {
        let mut c = card("read", ToolCallStatus::Error);
        c.detail = Some("file not found".to_string());
        let raw = joined(&c, 80);
        assert!(raw.contains(RED));
        assert!(strip_ansi(&raw).contains("file not found"));
    }

    #[test]
    fn running_card_has_no_body() {
        let mut c = card("bash", ToolCallStatus::Running);
        c.summary = "command=sleep 1".to_string();
        c.detail = Some("should not show".to_string());
        let text = strip_ansi(&joined(&c, 80));
        assert!(!text.contains("should not show"));
    }

    #[test]
    fn every_line_within_width() {
        let mut c = card("grep", ToolCallStatus::Success);
        let lines: Vec<String> = (0..5)
            .map(|i| format!("src/very/long/path/that/keeps/going/file{i}.rs:{i}: {}", "x".repeat(120)))
            .collect();
        c.detail = Some(lines.join("\n"));
        for line in render_tool_card(&c, 50) {
            assert!(visible_width(&line) <= 50, "line exceeds width: {line:?}");
        }
    }

    // ---- layout structure (the 6a tool-card visual pass) ----

    /// The first rendered line is always blank, separating the card from the
    /// preceding block so adjacent cards never visually merge.
    #[test]
    fn card_starts_with_blank_separator_line() {
        let mut c = card("read_file", ToolCallStatus::Success);
        c.summary = "path=src/main.rs".to_string();
        let rendered = render_tool_card(&c, 60);
        assert!(!rendered.is_empty());
        assert_eq!(strip_ansi(&rendered[0]).trim(), "", "first line should be blank");
    }

    /// The header carries the themed status glyph (green ✓ on success), a short
    /// type tag, and the bold tool name — not the old `name — summary` form.
    #[test]
    fn header_has_colored_glyph_tag_and_bold_name() {
        let mut c = card("list_dir", ToolCallStatus::Success);
        c.summary = "path=.".to_string();
        let rendered = render_tool_card(&c, 70);
        // header is the line right after the blank separator
        let header = &rendered[1];
        assert!(header.contains(GREEN), "success glyph should be green: {header:?}");
        assert!(header.contains(BOLD), "tool name should be bold: {header:?}");
        let plain = strip_ansi(header);
        assert!(plain.contains('✓'), "{plain}");
        assert!(plain.contains("list"), "tag present: {plain}");
        assert!(plain.contains("list_dir"), "tool name present: {plain}");
        // no leftover em-dash separator from the old layout
        assert!(!plain.contains('—'), "old em-dash layout removed: {plain}");
    }

    /// Error glyph is red.
    #[test]
    fn error_header_glyph_is_red() {
        let mut c = card("read", ToolCallStatus::Error);
        c.detail = Some("boom".to_string());
        let rendered = render_tool_card(&c, 60);
        assert!(rendered[1].contains(RED), "error glyph should be red: {:?}", rendered[1]);
        assert!(strip_ansi(&rendered[1]).contains('✗'));
    }

    /// Every non-blank body line sits under the dim left gutter (`│`), so the
    /// card reads as one grouped unit. The header line does NOT carry a gutter.
    #[test]
    fn body_lines_carry_left_gutter() {
        let (dir, ws) = scratch();
        std::fs::create_dir_all(dir.join("sub")).expect("mkdir sub");
        std::fs::write(dir.join("alpha.rs"), "a").expect("write alpha");
        let out = crate::native_tools::list_dir(
            &ws,
            &serde_json::json!({ "path": dir.to_string_lossy() }),
        )
        .expect("list_dir");
        let c = text_tool_card("ls", "path=.", out);

        let rendered = render_tool_card(&c, 80);
        // lines[0] blank, lines[1] header (no gutter), lines[2..] body (gutter)
        assert!(!strip_ansi(&rendered[1]).starts_with('│'), "header has no gutter");
        let body: Vec<&String> = rendered[2..].iter().collect();
        assert!(!body.is_empty(), "expected body lines");
        for line in &body {
            assert!(
                strip_ansi(line).starts_with('│'),
                "body line should start with gutter: {line:?}"
            );
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The diff body keeps green `+` / red `-` colors under the gutter.
    #[test]
    fn diff_body_is_guttered_and_colored() {
        let mut c = card("edit_file", ToolCallStatus::Success);
        c.structured_content = Some(serde_json::json!({
            "diff": "@@ -1 +1 @@\n-old line\n+new line",
        }));
        let rendered = render_tool_card(&c, 80);
        let raw = rendered.join("\n");
        assert!(raw.contains(GREEN) && raw.contains(RED), "diff keeps +/- colors");
        // each diff line is guttered
        for line in &rendered[2..] {
            assert!(strip_ansi(line).starts_with('│'), "diff line guttered: {line:?}");
        }
        // no raw JSON braces leaked into the card
        assert!(!strip_ansi(&raw).contains('{'), "no JSON braces: {raw}");
    }

    /// The bash command line renders the cyan `$` prompt under the gutter.
    #[test]
    fn bash_command_line_guttered_with_prompt() {
        let mut c = card("bash", ToolCallStatus::Success);
        c.summary = "command=echo hi".to_string();
        c.detail = Some("hi".to_string());
        let rendered = render_tool_card(&c, 80);
        let cmd_line = rendered
            .iter()
            .find(|l| {
                let plain = strip_ansi(l);
                plain.starts_with('│') && plain.contains("$ echo hi")
            })
            .expect("command line present");
        assert!(strip_ansi(cmd_line).starts_with('│'), "command line guttered: {cmd_line:?}");
        assert!(strip_ansi(cmd_line).contains('$'), "shows $ prompt: {cmd_line:?}");
    }
}
