//! Minimal Handlebars-subset renderer for the OMP prompt templates copied into
//! `sub_agent/prompts/`. Supports `{{x}}`, `{{{x}}}`, dotted paths, `@root.x`,
//! `{{#if}}`/`{{else}}`/`{{/if}}`, `{{#unless}}`, `{{#each}}` (with `this`,
//! `@index`, `@last`). Nothing is HTML-escaped: output is model text.

use serde_json::Value;

/// Render `template` against `ctx` (a JSON object).
pub fn render(template: &str, ctx: &Value) -> String {
    let tokens = tokenize(template);
    let mut pos = 0;
    let nodes = parse(&tokens, &mut pos);
    let mut out = String::new();
    emit(&nodes, &Scope { current: ctx, root: ctx, index: 0, last: false }, &mut out);
    out
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
            Value::Array(items) => segment
                .parse::<usize>()
                .ok()
                .and_then(|i| items.get(i))
                .unwrap_or(&Value::Null),
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
        assert_eq!(two, " jobs:[A]r,[B]s!", "`.length` is not a JSON field; callers pass counts explicitly");
        assert_eq!(render("{{{raw}}} {{n}}", &json!({"raw": "<x>", "n": 3})), "<x> 3");
    }
}

/// Task description: `asyncGuidance`, `defaultAgent`, `scoutAvailable`,
/// `spawningDisabled`, `hasModelMentions`, `ircEnabled`, and `agentList`.
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
/// The upstream `wait` description; no variables.
pub const WAIT_TOOL: &str = include_str!("prompts/wait.md");
/// Worker system prompt: `agent`, `context`, `planReference`,
/// `planReferencePath`, `worktree`, `ircSelfId`, `ircPeers` (each has `id`,
/// `displayName`, `kind`, `status`, `activity`), `ircOmittedCount`, and
/// `ircParkedCount`.
pub const SUBAGENT_SYSTEM: &str = include_str!("prompts/subagent-system-prompt.md");
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
