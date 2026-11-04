//! Minimal Handlebars-subset renderer for the OMP prompt templates copied into
//! `sub_agent/prompts/`. Supports `{{x}}`, `{{{x}}}`, dotted paths, `@root.x`,
//! `{{#if}}`/`{{else}}`/`{{/if}}`, `{{#unless}}`, `{{#each}}` (with `this`,
//! `@index`, `@last`). Nothing is HTML-escaped: output is model text.

use std::borrow::Cow;

use serde_json::Value;

/// Render `template` against `ctx` (a JSON object).
pub fn render(template: &str, ctx: &Value) -> String {
    let tokens = tokenize(template);
    let mut pos = 0;
    let nodes = parse(&tokens, &mut pos);
    let mut out = String::new();
    emit(&nodes, &Scope { current: ctx, root: ctx, index: 0, last: false }, &mut out);
    format_rendered(&out)
}

// OMP formats the rendered text, rather than formatting the template before
// interpolation, so conditional branches cannot leave runs of empty lines.
fn format_rendered(rendered: &str) -> String {
    let lines: Vec<_> = rendered.split('\n').collect();
    let mut out: Vec<Cow<'_, str>> = Vec::with_capacity(lines.len());
    let mut in_code = false;
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i].trim_end();
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_code = !in_code;
            out.push(Cow::Borrowed(line));
            i += 1;
            continue;
        }
        if in_code {
            out.push(Cow::Borrowed(line));
            i += 1;
            continue;
        }
        if trimmed.is_empty() {
            if i + 1 == lines.len() || lines[i + 1].trim().is_empty() {
                while out.last().is_some_and(|line| line.is_empty()) {
                    out.pop();
                }
                i += 1;
                while i < lines.len() && lines[i].trim().is_empty() {
                    i += 1;
                }
                continue;
            }
            if !out.is_empty() && out.last().is_some_and(|line| !line.is_empty()) {
                out.push(Cow::Borrowed(""));
            }
            i += 1;
            continue;
        }
        let closing = trimmed.strip_prefix("</").and_then(|s| s.strip_suffix('>'))
            .is_some_and(|name| !name.is_empty() && name.bytes().all(|b| b.is_ascii_lowercase() || b == b'-' || b == b'_'));
        if closing {
            while out.last().is_some_and(|line| line.is_empty()) {
                out.pop();
            }
        }
        if trimmed.starts_with('|') && trimmed.ends_with('|') {
            let indent = &line[..line.len() - trimmed.len()];
            let is_separator = trimmed.bytes().all(|b| matches!(b, b'|' | b'-' | b':' | b' ' | b'\t'));
            let cells = trimmed.split('|').map(str::trim);
            let table = if is_separator {
                let cells = cells.filter(|cell| !cell.is_empty()).map(|cell| {
                    match (cell.starts_with(':'), cell.ends_with(':')) {
                        (true, true) => ":---:",
                        (true, false) => ":---",
                        (false, true) => "---:",
                        (false, false) => "---",
                    }
                }).collect::<Vec<_>>().join("|");
                format!("{indent}|{cells}|")
            } else {
                format!("{indent}{}", cells.collect::<Vec<_>>().join("|"))
            };
            out.push(Cow::Owned(table));
        } else {
            out.push(Cow::Borrowed(line));
        }
        i += 1;
    }
    while out.last().is_some_and(|line| line.is_empty()) {
        out.pop();
    }
    let mut result = String::with_capacity(rendered.len());
    for (index, line) in out.iter().enumerate() {
        if index != 0 {
            result.push('\n');
        }
        result.push_str(line);
    }
    result
}

enum Token<'a> {
    Text(&'a str),
    Tag(&'a str),
}

fn tokenize(template: &str) -> Vec<Token<'_>> {
    let mut tokens = Vec::new();
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        if start > 0 {
            tokens.push(Token::Text(&rest[..start]));
        }
        let (open, close) = if rest[start..].starts_with("{{{") { (3, "}}}") } else { (2, "}}") };
        let Some(end) = rest[start + open..].find(close) else {
            tokens.push(Token::Text(&rest[start..]));
            return tokens;
        };
        tokens.push(Token::Tag(rest[start + open..start + open + end].trim()));
        rest = &rest[start + open + end + close.len()..];
    }
    if !rest.is_empty() {
        tokens.push(Token::Text(rest));
    }
    tokens
}

enum Node<'a> {
    Text(&'a str),
    Var(&'a str),
    If { path: &'a str, negate: bool, then: Vec<Node<'a>>, otherwise: Vec<Node<'a>> },
    Each { path: &'a str, body: Vec<Node<'a>> },
}

/// Parse until a closing `{{/…}}` or `{{else}}` (left unconsumed for the caller).
fn parse<'a>(tokens: &[Token<'a>], pos: &mut usize) -> Vec<Node<'a>> {
    let mut nodes = Vec::new();
    while *pos < tokens.len() {
        match tokens[*pos] {
            Token::Text(text) => {
                nodes.push(Node::Text(text));
                *pos += 1;
            }
            Token::Tag(tag) if tag.starts_with('/') || tag == "else" => return nodes,
            Token::Tag(tag) => {
                *pos += 1;
                if let Some(rest) = tag.strip_prefix('#') {
                    let (kind, path) = rest.split_once(' ').unwrap_or((rest, ""));
                    let path = path.trim();
                    let then = parse(tokens, pos);
                    let mut otherwise = Vec::new();
                    if matches!(tokens.get(*pos), Some(Token::Tag("else"))) {
                        *pos += 1;
                        otherwise = parse(tokens, pos);
                    }
                    *pos += 1; // closing tag
                    nodes.push(match kind {
                        "each" => Node::Each { path, body: then },
                        "unless" => Node::If { path, negate: true, then, otherwise },
                        _ => Node::If { path, negate: false, then, otherwise },
                    });
                } else if !tag.starts_with('!') {
                    nodes.push(Node::Var(tag));
                }
            }
        }
    }
    nodes
}

struct Scope<'a> {
    current: &'a Value,
    root: &'a Value,
    index: usize,
    last: bool,
}

fn lookup(scope: &Scope<'_>, path: &str) -> Value {
    match path {
        "@index" => return Value::from(scope.index),
        "@last" => return Value::Bool(scope.last),
        "this" | "." => return scope.current.clone(),
        _ => {}
    }
    let (mut value, rest) = if let Some(rest) = path.strip_prefix("@root.") {
        (scope.root, rest)
    } else if let Some(rest) = path.strip_prefix("this.") {
        (scope.current, rest)
    } else {
        (scope.current, path)
    };
    for segment in rest.split('.') {
        let segment = segment.trim_start_matches('[').trim_end_matches(']');
        value = match value {
            Value::Object(map) => map.get(segment).unwrap_or(&Value::Null),
            Value::Array(items) if segment == "length" => return Value::from(items.len()),
            Value::Array(items) => segment.parse::<usize>().ok()
                .and_then(|i| items.get(i)).unwrap_or(&Value::Null),
            _ => &Value::Null,
        };
    }
    value.clone()
}

fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(_) => true,
    }
}

fn emit(nodes: &[Node<'_>], scope: &Scope<'_>, out: &mut String) {
    for node in nodes {
        match node {
            Node::Text(text) => out.push_str(text),
            Node::Var(path) => match lookup(scope, path) {
                Value::Null => {}
                Value::String(s) => out.push_str(&s),
                other => out.push_str(&other.to_string()),
            },
            Node::If { path, negate, then, otherwise } => {
                let branch = if truthy(&lookup(scope, path)) != *negate { then } else { otherwise };
                emit(branch, scope, out);
            }
            Node::Each { path, body } => {
                if let Value::Array(items) = lookup(scope, path) {
                    let count = items.len();
                    for (index, item) in items.iter().enumerate() {
                        let inner = Scope { current: item, root: scope.root, index, last: index + 1 == count };
                        emit(body, &inner, out);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn renders_conditionals_loops_and_root_paths() {
        let template = "{{#if multiple}}{{jobs.length}} jobs{{else}}Job {{jobs.[0].id}}{{/if}}:{{#each jobs}}{{#if @root.multiple}}[{{this.id}}]{{/if}}{{this.result}}{{#unless @last}},{{/unless}}{{/each}}{{#unless missing}}!{{/unless}}";
        let one = render(template, &json!({"multiple": false, "jobs": [{"id": "A", "result": "r"}]}));
        assert_eq!(one, "Job A:r!");
        let two = render(template, &json!({"multiple": true, "jobs": [{"id": "A", "result": "r"}, {"id": "B", "result": "s"}]}));
        assert_eq!(two, "2 jobs:[A]r,[B]s!");
        assert_eq!(render("{{{raw}}} {{n}}", &json!({"raw": "<x>", "n": 3})), "<x> 3");
    }

    #[test]
    fn normalizes_rendered_whitespace_without_collapsing_code_blocks() {
        let template = "<section>  \nA  \n{{#if optional}}{{optional}}{{/if}}\n\nB\n\n</section>\n\n```\none\n\n\n two  \n```\n";
        assert_eq!(render(template, &json!({})), "<section>\nA\nB\n</section>\n\n```\none\n\n\n two\n```");
        assert_eq!(render("| one | two |\n| :------ | ----: |\n", &json!({})), "|one|two|\n|:---|---:|");
    }

    #[test]
    fn actual_templates_switch_between_sync_async_and_plan_content() {
        let guidance = render(TASK_ASYNC_CONTRACT, &json!({"waitTool": true, "ircEnabled": false}));
        let async_text = render(TASK_TOOL, &json!({
            "asyncEnabled": true, "hasBlockingAgents": true, "asyncGuidance": guidance,
            "defaultAgent": "task", "ircEnabled": false
        }));
        assert!(async_text.contains("BLOCKING agents return inline"));
        assert!(async_text.contains("Call `wait`"));
        assert!(!async_text.contains("proc://"));
        assert!(!async_text.contains("write agent://"));
        assert!(!async_text.contains("local://"));
        let sync_text = render(TASK_TOOL, &json!({"asyncEnabled": false, "defaultAgent": "task"}));
        assert!(sync_text.contains("Run `tasks[]` synchronously"));
        assert!(!sync_text.contains("Results auto-deliver"));
        let worker = render(SUBAGENT_SYSTEM, &json!({
            "agent": "Planner", "context": "shared", "planReference": "The approved plan.",
            "planReferencePath": "plan.md", "ircSelfId": ""
        }));
        assert!(worker.contains("<plan path=\"plan.md\">\nThe approved plan.\n</plan>"));
        assert!(!worker.contains("# Peers"));
        assert!(worker.contains("No `yield` tool exists"));
        assert!(PLAN_MODE_SUBAGENT.contains("### Critical Files for Implementation"));
    }

    #[test]
    fn completion_probe_renders_inflight_details() {
        let prompt = render(COMPLETION_PROBE, &json!({
            "inflight": [{"name": "grep", "path": "src", "chars": 12}],
            "previous": {"ago": "5m", "percent": 50, "stage": "reading"}
        }));
        assert!(prompt.contains("`grep` (src): 12 characters"));
        assert!(prompt.contains("5m ago, was 50%"));
        assert!(!render(COMPLETION_PROBE, &json!({"inflight": []})).contains("mid-response"));
    }
}

/// Task description: `asyncEnabled`, `hasBlockingAgents`, `asyncGuidance`,
/// `defaultAgent`, `scoutAvailable`, `spawningDisabled`, `hasModelMentions`,
/// `ircEnabled`, and `agentList`.
/// `agentList` is preformatted OMP agent-list text, including its trailing newline.
pub const TASK_TOOL: &str = include_str!("prompts/task.md");
/// Async guidance: `waitTool` and `ircEnabled`.
pub const TASK_ASYNC_CONTRACT: &str = include_str!("prompts/task-async-contract.md");
/// Spawn feedback: `mixed`, `count`, `singular`, `scheduleFailureSummary`,
/// `singleCall`, `agentId`, `jobId`, `agentLabel`, `showListing`, and `started`
/// (items have `agentId` and `jobId`).
pub const TASK_SPAWN_FEEDBACK: &str = include_str!("prompts/task-spawn-feedback.md");
/// Result envelope: `id`, `agentName`, `status`, `duration`, `truncated`,
/// `meta.lineCount`, `meta.charSize`, `abortReason`, `resumable`, `error`,
/// `preview`, and `mergeSummary`.
pub const TASK_SUMMARY: &str = include_str!("prompts/task-summary.md");
/// Follow-up: `agentId`, `aborted`, `resumable`, `ircEnabled`, and
/// `transcriptAvailable`.
pub const TASK_FOLLOW_UP: &str = include_str!("prompts/task-follow-up.md");
/// Delivery envelope: `multiple`, `jobCount`, and `jobs` (items have `jobId`,
/// `label`, `result`, `showAgentUrl`, `showHistoryUrl`, and `agentUrlId`).
pub const ASYNC_RESULT: &str = include_str!("prompts/async-result.md");
/// Stage-one background-job wait description; no variables.
pub const WAIT_TOOL: &str = include_str!("prompts/wait.md");
/// Worker system prompt: `agent`, `context`, `planReference`,
/// `planReferencePath`, `worktree`, `ircSelfId`, `ircPeers` (each has `id`,
/// `displayName`, `kind`, `status`, `activity`), `ircOmittedCount`, and
/// `ircParkedCount`.
pub const SUBAGENT_SYSTEM: &str = include_str!("prompts/subagent-system-prompt.md");
/// Read-only planning persona used when the parent session is in plan mode.
pub const PLAN_MODE_SUBAGENT: &str = include_str!("prompts/plan-mode-subagent.md");
/// Worker user prompt: `assignment`.
pub const SUBAGENT_USER: &str = include_str!("prompts/subagent-user-prompt.md");
/// Non-structured budget-stop reminder; no variables.
pub const SUBAGENT_BUDGET_REMINDER: &str = include_str!("prompts/subagent-yield-reminder.md");
/// Completion probe: `inflight` (items have `name`, `path`, `chars`) and
/// `previous` (with `ago`, `percent`, `stage`).
pub const COMPLETION_PROBE: &str = include_str!("prompts/subagent-completion-probe.md");

/// OMP task agent body.
pub const AGENT_TASK: &str = include_str!("prompts/agents/task.md");
/// OMP scout agent body, without frontmatter.
pub const AGENT_SCOUT: &str = include_str!("prompts/agents/scout.md");
/// OMP reviewer agent body, without frontmatter.
pub const AGENT_REVIEWER: &str = include_str!("prompts/agents/reviewer.md");
/// OMP security reviewer body, adapted to non-structured results.
pub const AGENT_SECURITY_REVIEWER: &str = include_str!("prompts/agents/security-reviewer.md");
/// Historical consultant body; dispatch requires historical-session support.
pub const AGENT_SEANCE: &str = include_str!("prompts/agents/seance.md");

#[cfg(test)]
mod template_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn renders_task_summary_inside_async_delivery_envelope() {
        let summary = render(TASK_SUMMARY, &json!({
            "id": "CalmPanda", "agentName": "scout", "status": "completed",
            "duration": "2s", "preview": "Found the answer."
        }));
        assert_eq!(
            summary.trim(),
            "<task-result id=\"CalmPanda\" agent=\"scout\" status=\"completed\" duration=\"2s\">\n\n<output>\nFound the answer.\n</output>\n</task-result>"
        );
        let notice = render(ASYNC_RESULT, &json!({
            "multiple": false, "jobCount": 1,
            "jobs": [{"jobId": "job-1", "result": summary.trim()}]
        }));
        assert_eq!(
            notice.trim(),
            format!("<system-notice>\nBackground job job-1 has completed. Resume your work using the result below.\n{}\n</system-notice>", summary.trim())
        );
    }

    #[test]
    fn renders_multiple_job_count_and_labels() {
        let notice = render(ASYNC_RESULT, &json!({
            "multiple": true, "jobCount": 2,
            "jobs": [
                {"jobId": "one", "label": "Scout", "result": "first"},
                {"jobId": "two", "result": "second"}
            ]
        }));
        assert!(notice.contains("2 background jobs have completed."));
        assert!(notice.contains("── Job one (Scout) ──\nfirst"));
        assert!(notice.contains("── Job two ──\nsecond"));
    }
}
