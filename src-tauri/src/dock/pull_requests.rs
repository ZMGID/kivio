//! Dock「拉取请求」面板：经 GitHub CLI（gh）读取当前仓库的 PR 列表与详情，
//! 检出 PR 分支，为当前分支创建 PR。
//!
//! 列表用一次 GraphQL 请求取齐：查看者、仓库、当前分支的 PR 和一页 PR（含 CI 汇总状态）。
//! `gh pr list --json statusCheckRollup` 会拉取每个检查项，几十个 PR 时明显更慢。
//! 仓库由 gh 按当前目录的远程解析（`{owner}` / `{repo}` 占位符），与 `gh pr` 命令同一口径。

use serde::Serialize;
use serde_json::Value;

use super::git::{
    ensure_ready_state, git_push_sync, git_status_sync, operation_response, run_gh, trim_output,
    GitOperationResponse, GitOutput,
};

const PAGE_SIZE: u32 = 30;
const GH_SPAWN_FAILED: &str = "gh 执行失败";

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestLabel {
    pub name: String,
    /// 不带 `#` 的十六进制颜色。
    pub color: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestItem {
    pub number: u64,
    pub title: String,
    pub url: String,
    /// "open" | "draft" | "merged" | "closed"
    pub state: String,
    pub created_at: String,
    pub updated_at: String,
    pub additions: u64,
    pub deletions: u64,
    pub changed_files: u64,
    pub head_ref: String,
    pub base_ref: String,
    pub cross_repository: bool,
    pub author: String,
    pub author_avatar: String,
    pub labels: Vec<PullRequestLabel>,
    pub comments: u64,
    /// "success" | "failure" | "pending" | "none"
    pub checks: String,
    /// "approved" | "changes_requested" | "review_required" | ""
    pub review_decision: String,
    pub conflicting: bool,
    /// 当前登录账号在待审查名单里。
    pub review_requested: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestList {
    /// "ready" | "not_repo" | "gh_missing" | "gh_unauthenticated" | "no_github_remote"
    pub status: String,
    pub hint: String,
    pub viewer: String,
    pub repository: String,
    pub repository_url: String,
    pub default_branch: String,
    /// 本地当前分支；游离 HEAD 时为空。
    pub current_branch: String,
    /// 当前分支最近一次的 PR（任意状态），只在第一页返回。
    pub current: Option<PullRequestItem>,
    pub total_count: u64,
    pub items: Vec<PullRequestItem>,
    pub end_cursor: Option<String>,
    pub has_more: bool,
}

impl PullRequestList {
    fn unavailable(status: &str, hint: &str) -> Self {
        Self {
            status: status.to_string(),
            hint: hint.to_string(),
            viewer: String::new(),
            repository: String::new(),
            repository_url: String::new(),
            default_branch: String::new(),
            current_branch: String::new(),
            current: None,
            total_count: 0,
            items: Vec::new(),
            end_cursor: None,
            has_more: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestCheck {
    pub name: String,
    pub workflow: String,
    /// "success" | "failure" | "pending" | "skipped"
    pub status: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestReview {
    pub author: String,
    /// "approved" | "changes_requested" | "commented" | "dismissed" | "pending"
    pub state: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestFile {
    pub path: String,
    pub additions: u64,
    pub deletions: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestDetail {
    pub number: u64,
    pub body: String,
    pub checks: Vec<PullRequestCheck>,
    pub reviews: Vec<PullRequestReview>,
    pub files: Vec<PullRequestFile>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestCreated {
    pub url: String,
}

// ---- gh 调用与错误分类 ----

enum GhFailure {
    Missing,
    Unauthenticated,
    NoGithubRemote,
    Other(String),
}

fn classify_gh_error(message: &str) -> GhFailure {
    let lower = message.to_ascii_lowercase();
    if message.starts_with(GH_SPAWN_FAILED) {
        GhFailure::Missing
    } else if lower.contains("gh auth login")
        || lower.contains("not logged into")
        || lower.contains("authentication required")
        || lower.contains("bad credentials")
    {
        GhFailure::Unauthenticated
    } else if lower.contains("none of the git remotes")
        || lower.contains("no git remotes")
        || lower.contains("unable to determine base repository")
        || lower.contains("could not determine base repo")
    {
        GhFailure::NoGithubRemote
    } else {
        GhFailure::Other(message.to_string())
    }
}

fn gh_json(repo_root: &str, args: &[&str]) -> Result<Value, GhFailure> {
    let output = run_gh(repo_root, args).map_err(|e| classify_gh_error(&e))?;
    if !output.status.success() {
        let stderr = trim_output(&output.stderr);
        let stdout = trim_output(&output.stdout);
        return Err(classify_gh_error(if stderr.is_empty() {
            &stdout
        } else {
            &stderr
        }));
    }
    let value: Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| GhFailure::Other(format!("无法解析 gh 输出：{e}")))?;
    // GraphQL 局部错误：有 errors 且没有 data 时整体失败。
    if let Some(errors) = value.get("errors").and_then(Value::as_array) {
        if value.get("data").is_none_or(Value::is_null) {
            let message = errors
                .iter()
                .filter_map(|error| error.get("message").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("；");
            return Err(classify_gh_error(&message));
        }
    }
    Ok(value)
}

fn failure_message(failure: GhFailure) -> String {
    match failure {
        GhFailure::Missing => "没有找到 GitHub CLI（gh），请先安装。".to_string(),
        GhFailure::Unauthenticated => {
            "GitHub CLI 未登录，请先在终端运行 gh auth login。".to_string()
        }
        GhFailure::NoGithubRemote => "仓库没有指向 GitHub 的远程。".to_string(),
        GhFailure::Other(message) => message,
    }
}

// ---- JSON → 契约 ----

fn str_at<'a>(value: &'a Value, pointer: &str) -> &'a str {
    value.pointer(pointer).and_then(Value::as_str).unwrap_or("")
}

fn u64_at(value: &Value, pointer: &str) -> u64 {
    value.pointer(pointer).and_then(Value::as_u64).unwrap_or(0)
}

fn nodes<'a>(value: &'a Value, pointer: &str) -> impl Iterator<Item = &'a Value> {
    value
        .pointer(pointer)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
}

fn rollup_state(raw: &str) -> &'static str {
    match raw {
        "SUCCESS" => "success",
        "FAILURE" | "ERROR" => "failure",
        "PENDING" | "EXPECTED" => "pending",
        _ => "none",
    }
}

fn pull_request_from_graphql(node: &Value, viewer: &str) -> Option<PullRequestItem> {
    let number = node.get("number")?.as_u64()?;
    let state = match (
        str_at(node, "/state"),
        node.get("isDraft").and_then(Value::as_bool),
    ) {
        ("MERGED", _) => "merged",
        ("CLOSED", _) => "closed",
        (_, Some(true)) => "draft",
        _ => "open",
    };
    let checks = rollup_state(str_at(
        node,
        "/commits/nodes/0/commit/statusCheckRollup/state",
    ));
    let review_decision = match str_at(node, "/reviewDecision") {
        "APPROVED" => "approved",
        "CHANGES_REQUESTED" => "changes_requested",
        "REVIEW_REQUIRED" => "review_required",
        _ => "",
    };
    let review_requested = !viewer.is_empty()
        && nodes(node, "/reviewRequests/nodes").any(|request| {
            str_at(request, "/requestedReviewer/login").eq_ignore_ascii_case(viewer)
        });
    Some(PullRequestItem {
        number,
        title: str_at(node, "/title").to_string(),
        url: str_at(node, "/url").to_string(),
        state: state.to_string(),
        created_at: str_at(node, "/createdAt").to_string(),
        updated_at: str_at(node, "/updatedAt").to_string(),
        additions: u64_at(node, "/additions"),
        deletions: u64_at(node, "/deletions"),
        changed_files: u64_at(node, "/changedFiles"),
        head_ref: str_at(node, "/headRefName").to_string(),
        base_ref: str_at(node, "/baseRefName").to_string(),
        cross_repository: node
            .get("isCrossRepository")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        author: str_at(node, "/author/login").to_string(),
        author_avatar: str_at(node, "/author/avatarUrl").to_string(),
        labels: nodes(node, "/labels/nodes")
            .map(|label| PullRequestLabel {
                name: str_at(label, "/name").to_string(),
                color: str_at(label, "/color").to_string(),
            })
            .filter(|label| !label.name.is_empty())
            .collect(),
        comments: u64_at(node, "/comments/totalCount"),
        checks: checks.to_string(),
        review_decision: review_decision.to_string(),
        conflicting: matches!(state, "open" | "draft")
            && str_at(node, "/mergeable") == "CONFLICTING",
        review_requested,
    })
}

fn check_from_rollup(entry: &Value) -> PullRequestCheck {
    let is_status_context = str_at(entry, "/__typename") == "StatusContext";
    let status = if is_status_context {
        match str_at(entry, "/state") {
            "SUCCESS" => "success",
            "FAILURE" | "ERROR" => "failure",
            _ => "pending",
        }
    } else if str_at(entry, "/status") != "COMPLETED" {
        "pending"
    } else {
        match str_at(entry, "/conclusion") {
            "SUCCESS" => "success",
            "SKIPPED" | "NEUTRAL" | "STALE" => "skipped",
            _ => "failure",
        }
    };
    let (name, url) = if is_status_context {
        (str_at(entry, "/context"), str_at(entry, "/targetUrl"))
    } else {
        (str_at(entry, "/name"), str_at(entry, "/detailsUrl"))
    };
    PullRequestCheck {
        name: name.to_string(),
        workflow: str_at(entry, "/workflowName").to_string(),
        status: status.to_string(),
        url: url.to_string(),
    }
}

fn detail_from_view(number: u64, value: &Value) -> PullRequestDetail {
    let mut checks: Vec<PullRequestCheck> = nodes(value, "/statusCheckRollup")
        .map(check_from_rollup)
        .collect();
    // 失败与进行中排在前面：用户展开详情时最关心哪项没过。
    let rank = |status: &str| match status {
        "failure" => 0,
        "pending" => 1,
        "success" => 2,
        _ => 3,
    };
    checks.sort_by(|a, b| {
        rank(&a.status)
            .cmp(&rank(&b.status))
            .then(a.name.cmp(&b.name))
    });
    PullRequestDetail {
        number,
        body: str_at(value, "/body").to_string(),
        checks,
        reviews: nodes(value, "/latestReviews")
            .map(|review| PullRequestReview {
                author: str_at(review, "/author/login").to_string(),
                state: str_at(review, "/state").to_ascii_lowercase(),
            })
            .filter(|review| !review.author.is_empty())
            .collect(),
        files: nodes(value, "/files")
            .map(|file| PullRequestFile {
                path: str_at(file, "/path").to_string(),
                additions: u64_at(file, "/additions"),
                deletions: u64_at(file, "/deletions"),
            })
            .collect(),
    }
}

// ---- 列表 ----

const PR_FIELDS: &str = "number title url state isDraft createdAt updatedAt \
additions deletions changedFiles headRefName baseRefName isCrossRepository reviewDecision mergeable \
author { login avatarUrl(size: 48) } labels(first: 6) { nodes { name color } } comments { totalCount } \
reviewRequests(first: 10) { nodes { requestedReviewer { ... on User { login } } } } \
commits(last: 1) { nodes { commit { statusCheckRollup { state } } } }";

fn states_literal(state: &str) -> Result<&'static str, String> {
    match state {
        "open" => Ok("[OPEN]"),
        "merged" => Ok("[MERGED]"),
        "closed" => Ok("[CLOSED]"),
        "all" => Ok("[OPEN, MERGED, CLOSED]"),
        other => Err(format!("未知的 PR 状态筛选：{other}")),
    }
}

fn list_query(states: &str, with_current: bool) -> String {
    let (branch_var, current) = if with_current {
        (
            ", $branch: String!",
            "current: pullRequests(headRefName: $branch, first: 5, states: [OPEN, MERGED, CLOSED], \
orderBy: {field: UPDATED_AT, direction: DESC}) { nodes { ...Pr headRepositoryOwner { login } } }",
        )
    } else {
        ("", "")
    };
    format!(
        "query($owner: String!, $repo: String!, $after: String{branch_var}) {{ \
viewer {{ login }} \
repository(owner: $owner, name: $repo) {{ nameWithOwner url defaultBranchRef {{ name }} {current} \
list: pullRequests(first: {PAGE_SIZE}, after: $after, states: {states}, \
orderBy: {{field: UPDATED_AT, direction: DESC}}) {{ totalCount pageInfo {{ hasNextPage endCursor }} nodes {{ ...Pr }} }} }} }} \
fragment Pr on PullRequest {{ {PR_FIELDS} }}"
    )
}

fn list_from_graphql(value: &Value, current_branch: &str) -> PullRequestList {
    let data = value.get("data").unwrap_or(&Value::Null);
    let viewer = str_at(data, "/viewer/login").to_string();
    let repo = data.get("repository").unwrap_or(&Value::Null);
    let items = nodes(repo, "/list/nodes")
        .filter_map(|node| pull_request_from_graphql(node, &viewer))
        .collect();
    // 只认本仓库或查看者自己 fork 的同名分支：别人 fork 里也可能有 `main`。
    let current = nodes(repo, "/current/nodes")
        .filter(|node| {
            node.get("isCrossRepository").and_then(Value::as_bool) != Some(true)
                || (!viewer.is_empty()
                    && str_at(node, "/headRepositoryOwner/login").eq_ignore_ascii_case(&viewer))
        })
        .find_map(|node| pull_request_from_graphql(node, &viewer));
    let end_cursor = repo
        .pointer("/list/pageInfo/endCursor")
        .and_then(Value::as_str)
        .map(str::to_string);
    PullRequestList {
        status: "ready".to_string(),
        hint: String::new(),
        repository: str_at(repo, "/nameWithOwner").to_string(),
        repository_url: str_at(repo, "/url").to_string(),
        default_branch: str_at(repo, "/defaultBranchRef/name").to_string(),
        current_branch: current_branch.to_string(),
        current,
        total_count: u64_at(repo, "/list/totalCount"),
        items,
        has_more: repo
            .pointer("/list/pageInfo/hasNextPage")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        end_cursor,
        viewer,
    }
}

fn current_branch_of(head: &str) -> &str {
    if head.is_empty() || head == "HEAD" || head == "(detached)" {
        ""
    } else {
        head
    }
}

fn pr_list_sync(
    workdir: &str,
    state: &str,
    after: Option<String>,
) -> Result<PullRequestList, String> {
    let repo_state = git_status_sync(workdir)?;
    if repo_state.status != "ready" {
        return Ok(PullRequestList::unavailable("not_repo", ""));
    }
    let branch = current_branch_of(&repo_state.head).to_string();
    let after = after
        .map(|cursor| cursor.trim().to_string())
        .filter(|c| !c.is_empty());
    let with_current = after.is_none() && !branch.is_empty();
    let query = list_query(states_literal(state)?, with_current);
    let query_arg = format!("query={query}");
    let mut args = vec![
        "api",
        "graphql",
        "-F",
        "owner={owner}",
        "-F",
        "repo={repo}",
        "-f",
        query_arg.as_str(),
    ];
    let branch_arg = format!("branch={branch}");
    if with_current {
        args.extend(["-f", branch_arg.as_str()]);
    }
    let after_arg = after.as_ref().map(|cursor| format!("after={cursor}"));
    if let Some(after_arg) = after_arg.as_deref() {
        args.extend(["-f", after_arg]);
    }
    match gh_json(&repo_state.repo_root, &args) {
        Ok(value) => Ok(list_from_graphql(&value, &branch)),
        Err(GhFailure::Missing) => Ok(PullRequestList::unavailable(
            "gh_missing",
            &failure_message(GhFailure::Missing),
        )),
        Err(GhFailure::Unauthenticated) => Ok(PullRequestList::unavailable(
            "gh_unauthenticated",
            &failure_message(GhFailure::Unauthenticated),
        )),
        Err(GhFailure::NoGithubRemote) => Ok(PullRequestList::unavailable(
            "no_github_remote",
            &failure_message(GhFailure::NoGithubRemote),
        )),
        Err(GhFailure::Other(message)) => Err(message),
    }
}

#[tauri::command]
pub async fn dock_pr_list(
    workdir: String,
    state: String,
    after: Option<String>,
) -> Result<PullRequestList, String> {
    tauri::async_runtime::spawn_blocking(move || pr_list_sync(&workdir, &state, after))
        .await
        .map_err(|e| format!("dock_pr_list join: {e}"))?
}

// ---- 详情 ----

fn pr_detail_sync(workdir: &str, number: u64) -> Result<PullRequestDetail, String> {
    let state = ensure_ready_state(workdir)?;
    let number_arg = number.to_string();
    let value = gh_json(
        &state.repo_root,
        &[
            "pr",
            "view",
            number_arg.as_str(),
            "--json",
            "body,statusCheckRollup,latestReviews,files",
        ],
    )
    .map_err(failure_message)?;
    Ok(detail_from_view(number, &value))
}

#[tauri::command]
pub async fn dock_pr_detail(workdir: String, number: u64) -> Result<PullRequestDetail, String> {
    tauri::async_runtime::spawn_blocking(move || pr_detail_sync(&workdir, number))
        .await
        .map_err(|e| format!("dock_pr_detail join: {e}"))?
}

// ---- 检出 ----

fn gh_operation(repo_root: &str, args: &[&str]) -> Result<GitOutput, String> {
    let output = run_gh(repo_root, args).map_err(|e| failure_message(classify_gh_error(&e)))?;
    let stdout = trim_output(&output.stdout);
    let stderr = trim_output(&output.stderr);
    if output.status.success() {
        Ok(GitOutput { stdout, stderr })
    } else {
        Err(failure_message(classify_gh_error(if stderr.is_empty() {
            &stdout
        } else {
            &stderr
        })))
    }
}

fn pr_checkout_sync(workdir: &str, number: u64) -> Result<GitOperationResponse, String> {
    let state = ensure_ready_state(workdir)?;
    let number_arg = number.to_string();
    let result = gh_operation(&state.repo_root, &["pr", "checkout", number_arg.as_str()]);
    operation_response(workdir, result, "已检出 PR 分支。")
}

#[tauri::command]
pub async fn dock_pr_checkout(
    workdir: String,
    number: u64,
) -> Result<GitOperationResponse, String> {
    tauri::async_runtime::spawn_blocking(move || pr_checkout_sync(&workdir, number))
        .await
        .map_err(|e| format!("dock_pr_checkout join: {e}"))?
}

// ---- 创建 ----

fn pr_create_sync(
    workdir: &str,
    title: String,
    body: String,
    base: String,
    draft: bool,
) -> Result<PullRequestCreated, String> {
    let state = ensure_ready_state(workdir)?;
    let branch = current_branch_of(&state.head).to_string();
    if branch.is_empty() {
        return Err("当前不在任何分支上，无法创建 PR。".to_string());
    }
    let title = title.trim();
    if title.is_empty() {
        return Err("PR 标题不能为空。".to_string());
    }
    let base = base.trim();
    if base.is_empty() || base == branch {
        return Err("请选择与当前分支不同的目标分支。".to_string());
    }
    // 远端还没有当前分支或有未推送的提交：先推送（首次推送会建立跟踪），再建 PR。
    if state.upstream.is_empty() || state.ahead > 0 {
        let pushed = git_push_sync(workdir)?;
        if !pushed.ok {
            return Err(pushed.message);
        }
    }
    let mut args = vec![
        "pr",
        "create",
        "--head",
        branch.as_str(),
        "--base",
        base,
        "--title",
        title,
        "--body",
        body.as_str(),
    ];
    if draft {
        args.push("--draft");
    }
    let output = gh_operation(&state.repo_root, &args)?;
    // gh 成功时最后一行是 PR 链接。
    let url = output
        .stdout
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| line.starts_with("https://"))
        .unwrap_or_default()
        .to_string();
    Ok(PullRequestCreated { url })
}

#[tauri::command]
pub async fn dock_pr_create(
    workdir: String,
    title: String,
    body: String,
    base: String,
    draft: bool,
) -> Result<PullRequestCreated, String> {
    tauri::async_runtime::spawn_blocking(move || pr_create_sync(&workdir, title, body, base, draft))
        .await
        .map_err(|e| format!("dock_pr_create join: {e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn node(overrides: Value) -> Value {
        let mut base = json!({
            "number": 70,
            "title": "feat: FreeRoute",
            "url": "https://github.com/o/r/pull/70",
            "state": "OPEN",
            "isDraft": false,
            "createdAt": "2026-10-23T07:19:10Z",
            "updatedAt": "2026-11-10T14:51:44Z",
            "additions": 1442,
            "deletions": 9,
            "changedFiles": 39,
            "headRefName": "feat/freeroute",
            "baseRefName": "main",
            "isCrossRepository": false,
            "reviewDecision": null,
            "mergeable": "MERGEABLE",
            "author": { "login": "octo", "avatarUrl": "https://avatars/x" },
            "labels": { "nodes": [{ "name": "bug", "color": "d73a4a" }] },
            "comments": { "totalCount": 2 },
            "reviewRequests": { "nodes": [{ "requestedReviewer": { "login": "Viewer" } }] },
            "commits": { "nodes": [{ "commit": { "statusCheckRollup": { "state": "FAILURE" } } }] }
        });
        if let (Some(target), Some(extra)) = (base.as_object_mut(), overrides.as_object()) {
            for (key, value) in extra {
                target.insert(key.clone(), value.clone());
            }
        }
        base
    }

    #[test]
    fn maps_graphql_pull_request() {
        let item = pull_request_from_graphql(&node(json!({})), "viewer").unwrap();
        assert_eq!(item.number, 70);
        assert_eq!(item.state, "open");
        assert_eq!(item.checks, "failure");
        assert_eq!(item.author, "octo");
        assert_eq!(
            item.labels,
            vec![PullRequestLabel {
                name: "bug".into(),
                color: "d73a4a".into()
            }]
        );
        assert_eq!(item.comments, 2);
        assert!(item.review_requested, "reviewer login match ignores case");
        assert!(!item.conflicting);
        assert_eq!(item.review_decision, "");
    }

    #[test]
    fn derives_state_checks_and_conflicts() {
        let draft = pull_request_from_graphql(
            &node(
                json!({ "isDraft": true, "mergeable": "CONFLICTING", "commits": { "nodes": [] } }),
            ),
            "",
        )
        .unwrap();
        assert_eq!(draft.state, "draft");
        assert_eq!(draft.checks, "none");
        assert!(draft.conflicting);
        assert!(!draft.review_requested, "no viewer means no review request");

        let merged = pull_request_from_graphql(
            &node(json!({ "state": "MERGED", "isDraft": true, "mergeable": "CONFLICTING", "reviewDecision": "APPROVED" })),
            "viewer",
        )
        .unwrap();
        assert_eq!(merged.state, "merged");
        assert!(!merged.conflicting, "merged PRs never show conflicts");
        assert_eq!(merged.review_decision, "approved");

        let pending = pull_request_from_graphql(
            &node(json!({ "commits": { "nodes": [{ "commit": { "statusCheckRollup": { "state": "EXPECTED" } } }] } })),
            "",
        )
        .unwrap();
        assert_eq!(pending.checks, "pending");
    }

    #[test]
    fn builds_list_with_current_branch_and_paging() {
        let value = json!({ "data": {
            "viewer": { "login": "viewer" },
            "repository": {
                "nameWithOwner": "o/r",
                "url": "https://github.com/o/r",
                "defaultBranchRef": { "name": "main" },
                "current": { "nodes": [
                    node(json!({ "number": 24, "isCrossRepository": true, "headRepositoryOwner": { "login": "stranger" } })),
                    node(json!({ "number": 9, "isCrossRepository": true, "headRepositoryOwner": { "login": "Viewer" } }))
                ] },
                "list": {
                    "totalCount": 31,
                    "pageInfo": { "hasNextPage": true, "endCursor": "Y3Vy" },
                    "nodes": [node(json!({})), { "title": "missing number" }]
                }
            }
        }});
        let list = list_from_graphql(&value, "feat/freeroute");
        assert_eq!(list.status, "ready");
        assert_eq!(list.viewer, "viewer");
        assert_eq!(list.repository, "o/r");
        assert_eq!(list.default_branch, "main");
        assert_eq!(list.current_branch, "feat/freeroute");
        assert_eq!(
            list.current.as_ref().map(|pr| pr.number),
            Some(9),
            "same-named branches in other forks are not the current branch"
        );
        assert_eq!(list.items.len(), 1, "nodes without a number are skipped");
        assert_eq!(list.total_count, 31);
        assert!(list.has_more);
        assert_eq!(list.end_cursor.as_deref(), Some("Y3Vy"));
    }

    #[test]
    fn query_only_asks_for_current_branch_on_first_page() {
        assert!(list_query("[OPEN]", true).contains("current: pullRequests(headRefName: $branch"));
        assert!(list_query("[OPEN]", true).contains("$branch: String!"));
        let later = list_query("[OPEN]", false);
        assert!(!later.contains("current:"));
        assert!(
            !later.contains("$branch"),
            "GraphQL rejects unused variables"
        );
        assert!(later.contains("states: [OPEN]"));
        assert!(
            states_literal("draft").is_err(),
            "draft is a client-side filter of open PRs"
        );
    }

    #[test]
    fn classifies_gh_failures() {
        assert!(matches!(
            classify_gh_error("gh 执行失败：program not found"),
            GhFailure::Missing
        ));
        assert!(matches!(
            classify_gh_error("To get started with GitHub CLI, please run:  gh auth login"),
            GhFailure::Unauthenticated
        ));
        assert!(matches!(
            classify_gh_error("none of the git remotes configured for this repository point to a known GitHub host"),
            GhFailure::NoGithubRemote
        ));
        assert!(matches!(classify_gh_error("HTTP 502"), GhFailure::Other(_)));
    }

    #[test]
    fn maps_detail_checks_failures_first() {
        let value = json!({
            "body": "## Why",
            "statusCheckRollup": [
                { "__typename": "CheckRun", "name": "quality", "workflowName": "CI", "status": "COMPLETED", "conclusion": "SUCCESS", "detailsUrl": "https://ci/1" },
                { "__typename": "CheckRun", "name": "windows", "workflowName": "CI", "status": "IN_PROGRESS", "conclusion": "", "detailsUrl": "https://ci/2" },
                { "__typename": "CheckRun", "name": "lint", "workflowName": "CI", "status": "COMPLETED", "conclusion": "TIMED_OUT", "detailsUrl": "https://ci/3" },
                { "__typename": "CheckRun", "name": "docs", "workflowName": "CI", "status": "COMPLETED", "conclusion": "SKIPPED", "detailsUrl": "" },
                { "__typename": "StatusContext", "context": "vercel", "state": "ERROR", "targetUrl": "https://v" }
            ],
            "latestReviews": [{ "author": { "login": "rev" }, "state": "CHANGES_REQUESTED" }],
            "files": [{ "path": "src/a.ts", "additions": 3, "deletions": 1 }]
        });
        let detail = detail_from_view(70, &value);
        let summary: Vec<(&str, &str)> = detail
            .checks
            .iter()
            .map(|check| (check.name.as_str(), check.status.as_str()))
            .collect();
        assert_eq!(
            summary,
            vec![
                ("lint", "failure"),
                ("vercel", "failure"),
                ("windows", "pending"),
                ("quality", "success"),
                ("docs", "skipped"),
            ]
        );
        assert_eq!(detail.checks[4].url, "");
        assert_eq!(
            detail.reviews,
            vec![PullRequestReview {
                author: "rev".into(),
                state: "changes_requested".into()
            }]
        );
        assert_eq!(detail.files[0].additions, 3);
        assert_eq!(detail.body, "## Why");
    }
}
