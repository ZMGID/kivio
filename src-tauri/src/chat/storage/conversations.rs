use super::index::conversation_file_ids_in_dir;
use super::migration::{
    conversation_has_project_binding, conversation_list_item_has_project_binding,
};
use super::*;

pub(crate) fn read_conversation_file(path: &Path, id: &str) -> Result<Conversation, String> {
    let content = fs::read_to_string(path).map_err(|e| format!("读取对话文件失败（{id}）：{e}"))?;
    serde_json::from_str(&content).map_err(|e| format!("对话文件已损坏，无法加载（{id}）：{e}"))
}

/// Only the identity needed to recheck a restart candidate under its keyed lock.
#[derive(Debug, Deserialize, PartialEq, Eq)]
pub(crate) struct RestartGoal {
    pub id: String,
    pub version: u64,
    pub status: crate::chat::GoalStatus,
}

/// Stop as soon as goal_state is decoded. New files put it before messages;
/// legacy files are streamed through IgnoredAny without allocating the history.
/// This is candidate discovery, not full-file validation: candidates are loaded
/// normally again under the repository lock before any change is persisted.
pub(super) fn read_restart_goal(reader: impl Read) -> Result<Option<RestartGoal>, String> {
    struct GoalVisitor<'a>(&'a mut Option<Option<RestartGoal>>);
    impl<'de> serde::de::Visitor<'de> for GoalVisitor<'_> {
        type Value = ();

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("a conversation object")
        }

        fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
            while let Some(key) = map.next_key::<String>()? {
                if key == "goal_state" {
                    *self.0 = Some(map.next_value()?);
                    // Returning Ok here would make serde_json expect the closing
                    // brace. This private sentinel intentionally cancels parsing;
                    // only a successfully decoded goal_state enables the fast exit.
                    return Err(serde::de::Error::custom("Goal header decoded"));
                }
                map.next_value::<serde::de::IgnoredAny>()?;
            }
            Ok(())
        }
    }

    let mut found = None;
    let mut deserializer = serde_json::Deserializer::from_reader(BufReader::new(reader));
    let result = serde::Deserializer::deserialize_map(&mut deserializer, GoalVisitor(&mut found));
    match found {
        Some(goal) => Ok(goal),
        None => result.map(|()| None).map_err(|error| error.to_string()),
    }
}

pub(crate) fn restart_goal_candidates(
    app: &AppHandle,
) -> Result<Vec<(String, RestartGoal)>, String> {
    restart_goal_candidates_in_dir(&conversations_dir(app)?)
}

pub(super) fn restart_goal_candidates_in_dir(
    dir: &Path,
) -> Result<Vec<(String, RestartGoal)>, String> {
    // Read the actual files, not index.json: a crash between the conversation
    // write and the index write must not conceal a running Goal.
    let mut candidates = Vec::new();
    for id in conversation_file_ids_in_dir(dir)? {
        let path = dir.join(format!("{id}.json"));
        let file = match fs::File::open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(error) => {
                eprintln!("skip unreadable Goal recovery file {id}: {error}");
                continue;
            }
        };
        match read_restart_goal(file) {
            Ok(Some(goal)) if crate::chat::goal::is_running(goal.status) => {
                candidates.push((id, goal))
            }
            Ok(_) => {}
            Err(error) => eprintln!("skip invalid Goal recovery file {id}: {error}"),
        }
    }
    Ok(candidates)
}

/// 加载对话详情
pub fn load_conversation(app: &AppHandle, id: &str) -> Result<Conversation, String> {
    let path = conversation_file_path(app, id)?;
    if !path.exists() {
        return Err(format!("对话不存在：{id}"));
    }

    read_conversation_file(&path, id)
}

fn write_conversation_file_at_path(
    path: &Path,
    conversation: Conversation,
) -> Result<Conversation, String> {
    // compact 而非 pretty：长对话数 MB 级,pretty 徒增 ~30-50% 体积与序列化时间,
    // 且每个工具轮都要整本重写。人读导出走 export.rs,不靠这份文件的排版。
    let content =
        serde_json::to_string(&conversation).map_err(|e| format!("serialize conversation: {e}"))?;
    atomic_write(path, &content, "conversation")?;
    Ok(conversation)
}

/// 保存对话详情
/// Low-level JSON store primitive. Conversation business code must write via
/// `ConversationRepository`; this function deliberately does not touch the index.
pub(crate) fn write_conversation_file(
    app: &AppHandle,
    mut conversation: Conversation,
) -> Result<Conversation, String> {
    let path = conversation_file_path(app, &conversation.id)?;
    // 外置内联图：artifact 的大图 + 两份隐藏转录（`model_messages` / `api_messages`）里
    // 模型看过的整图 base64。后者是"会话 JSON 绝不含 base64"的关键——它每轮都被整本读写，
    // 一张图存几份就是几 MB × 每轮 fsync。中断草稿同时持有两份转录，所以两个都要扫。
    // 参数取 owned（调用方本来就持有所有权），省掉此前每次落盘的整会话 clone。
    if conversation.messages.iter().any(|message| {
        crate::chat::attachments::message_has_inline_image_to_externalize(message)
            || crate::chat::attachments::message_has_model_message_image_to_externalize(message)
            || crate::chat::attachments::message_has_api_message_image_to_externalize(message)
    }) {
        let conv_id = conversation.id.clone();
        for message in conversation.messages.iter_mut() {
            crate::chat::attachments::externalize_message_artifacts(app, &conv_id, message);
        }
    }

    write_conversation_file_at_path(&path, conversation)
}

/// 删除对话。
///
/// **顺序即契约**：先摘掉「决定它还在不在侧栏」的两样东西——对话文件和索引条目——
/// 再清工作区 / 附件 / sandbox 导出这些副产物，且副产物删不掉只记警告、绝不中止。
///
/// 原来是反着来的（先 `remove_dir_all` 工作区，`?` 一路上抛），于是 Windows 上
/// 一个还在跑的 `npm run dev` 把 cwd 钉在 `chat-workspaces/<id>` 里，目录删不掉 →
/// 整个删除中止 → 对话文件和索引条目原封不动。而 `load_index_or_scan` 认「磁盘文件
/// 才是真相源」，下次刷新侧栏就把它重建回来了：用户看到的就是"删了又回来，点好几次
/// 才掉"。副产物残留顶多占点磁盘，比这个轻得多。
///
/// 返回未能清理的副产物说明（供上层提示用户），空 = 全清干净。
pub(crate) fn delete_conversation(app: &AppHandle, id: &str) -> Result<Vec<String>, String> {
    validate_conversation_id(id)?;
    let path = conversation_file_path(app, id)?;
    let mut index = load_index_or_scan(app)?;
    let indexed_item = index.conversations.iter().find(|item| item.id == id);

    // A missing conversation file must not prevent removing its stale index entry.
    // When metadata is also missing, stay conservative and leave any workbench alone.
    let remove_workspace = if path.exists() {
        // 文件读坏也不该拦住删除——读不出来就按「没绑项目」保守处理：不碰工作区。
        match load_conversation(app, id) {
            Ok(conversation) => !conversation_has_project_binding(app, &conversation)?,
            Err(_) => false,
        }
    } else if let Some(item) = indexed_item {
        !conversation_list_item_has_project_binding(app, item)?
    } else {
        false
    };
    // 工作区路径要在删文件之前解析（解析只读 settings，不碰磁盘），失败也只是不清工作区。
    let workspace = if remove_workspace {
        let state = app.state::<crate::state::AppState>();
        let settings = state.settings_read();
        let resolved = crate::native_tools::conversation_workspace_directory(
            &settings.chat_tools.native_tools.working_directory,
            id,
        );
        drop(settings);
        resolved.ok()
    } else {
        None
    };

    // ① 先断可见性：对话文件 + 索引条目。这两步失败才算删除失败。
    if path.exists() {
        fs::remove_file(&path).map_err(|e| format!("delete conversation file: {e}"))?;
    }
    index.conversations.retain(|c| c.id != id);
    save_index(app, &index)?;

    // ② 副产物尽力清，失败只记账。
    if let Ok(draft) = draft_journal_path(app, id) {
        let _ = fs::remove_file(draft);
    }
    let attachments_dir = match conversations_dir(app) {
        Ok(dir) => Some(dir.join(format!("{}_attachments", id))),
        Err(e) => {
            eprintln!("delete conversation {id}: attachments dir unavailable: {e}");
            None
        }
    };
    let mut warnings =
        remove_conversation_side_artifacts(workspace.as_deref(), attachments_dir.as_deref());

    // 外部 CLI 的会话绑定也要跟着走，否则那条原生会话会永远显示"已导入"、再也导不进来。
    // 只删 Kivio 侧的绑定记录，**不动 CLI 自己的 transcript**（用户在终端里还要 resume）。
    warnings.extend(crate::external_agents::session::remove_all_bindings(
        app, id,
    ));

    // Sweep legacy outputs/runs left by older versions. This never touches a project root.
    crate::native_tools::remove_sandbox_exports_for_conversation(id);

    Ok(warnings)
}

/// 删除对话的副产物目录（工作区 / 附件），**只回警告不回错**。
///
/// 单独抽出来是为了能脱离 `AppHandle` 单测这条不变式：任何一个目录删不掉，都不能
/// 变成整个删除失败——对话文件和索引已经在调用方那里先摘掉了，这里再抛错只会让
/// 上层以为删除没成功。
pub(super) fn remove_conversation_side_artifacts(
    workspace: Option<&Path>,
    attachments_dir: Option<&Path>,
) -> Vec<String> {
    let mut warnings = Vec::new();
    for (label, dir) in [("工作区", workspace), ("附件目录", attachments_dir)] {
        let Some(dir) = dir else { continue };
        if !dir.exists() {
            continue;
        }
        if !dir.is_dir() {
            warnings.push(format!("{label}不是目录，已跳过：{}", dir.display()));
            continue;
        }
        if let Err(e) = fs::remove_dir_all(dir) {
            warnings.push(format!("{label}未能清理（{}）：{e}", dir.display()));
        }
    }
    warnings
}

/// 获取对话列表（分页）。**默认排除已归档**（侧栏工作台不应出现归档对话）。
pub fn get_conversations(
    app: &AppHandle,
    offset: usize,
    limit: usize,
    folder: Option<String>,
    project_id: Option<String>,
    set_id: Option<String>,
) -> Result<Vec<ConversationListItem>, String> {
    let mut index = load_index_or_scan(app)?;
    // 侧栏 / 常规列表：归档对话不出现
    index.conversations.retain(|c| !c.archived);
    let set_filter = set_id.and_then(|id| {
        let trimmed = id.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    });
    let project_filter = project_id.and_then(|id| {
        let trimmed = id.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    });

    // 集与项目互斥：优先按 set_id 过滤；否则新项目按 project_id，旧对话回退 folder 名称。
    if let Some(set_id) = set_filter {
        index
            .conversations
            .retain(|c| c.set_id.as_deref() == Some(set_id.as_str()));
    } else if let Some(project_id) = project_filter {
        let fallback_folder = folder.as_deref();
        index.conversations.retain(|c| {
            c.project_id.as_deref() == Some(project_id.as_str())
                || (c.project_id.is_none() && c.folder.as_deref() == fallback_folder)
        });
    } else if let Some(folder_name) = folder {
        index
            .conversations
            .retain(|c| c.folder.as_deref() == Some(&folder_name));
    }

    // 按 updated_at 倒序排序（最新的在前）
    index
        .conversations
        .sort_by(|a, b| b.updated_at.cmp(&a.updated_at));

    // 分页
    if offset >= index.conversations.len() {
        return Ok(vec![]);
    }
    let end = (offset + limit).min(index.conversations.len());
    Ok(index.conversations[offset..end].to_vec())
}

pub(super) fn reusable_blank_index_matches(
    item: &ConversationListItem,
    provider_id: &str,
    model: &str,
    folder: Option<&str>,
    project_id: Option<&str>,
    set_id: Option<&str>,
    assistant_id: Option<&str>,
) -> bool {
    // 归档对话只出现在对话库「归档」书架。复用它当新对话，侧栏会在乐观行剪掉后把它吃掉：
    // 用户只能在生成中看到这条会话，结束后找不到。
    !item.archived
        && item.message_count == 0
        && item.provider_id == provider_id
        && item.model == model
        && item.folder.as_deref() == folder
        && item.project_id.as_deref() == project_id
        && item.set_id.as_deref() == set_id
        && item.assistant_id.as_deref() == assistant_id
}

pub fn find_reusable_blank_conversation(
    app: &AppHandle,
    provider_id: &str,
    model: &str,
    folder: Option<&str>,
    project_id: Option<&str>,
    set_id: Option<&str>,
    assistant_id: Option<&str>,
) -> Result<Option<Conversation>, String> {
    let mut index = load_index_or_scan(app)?;
    index
        .conversations
        .sort_by(|a, b| b.updated_at.cmp(&a.updated_at));

    for item in index.conversations {
        if !reusable_blank_index_matches(
            &item,
            provider_id,
            model,
            folder,
            project_id,
            set_id,
            assistant_id,
        ) {
            continue;
        }
        let conversation = match load_conversation(app, &item.id) {
            Ok(conversation) => conversation,
            Err(err) => {
                eprintln!("skip reusable blank conversation {}: {err}", item.id);
                continue;
            }
        };
        if conversation.archived {
            continue;
        }
        if conversation.messages.is_empty()
            && conversation.provider_id == provider_id
            && conversation.model == model
            && conversation.folder.as_deref() == folder
            && conversation.project_id.as_deref() == project_id
            && conversation.set_id.as_deref() == set_id
            && conversation.assistant_id.as_deref() == assistant_id
        {
            return Ok(Some(conversation));
        }
    }

    Ok(None)
}

/// 集/项目里对话的「钉住位置」。底座仍是更新时间倒序，被拖过的对话钉在 `row` 行，
/// 其余按时间填剩下的空位。显示顺序在前端算（嵌套列表本来就是前端拼的），
/// 后端只负责存 —— 所以这里没有排序逻辑，只有一份 group_id → 钉子表。
///
/// 单独一个文件而不是加到 ChatProject/ChatSet 上：那两个结构有多处构造点，
/// 加字段要挨个改且会动到已有序列化；钉子是纯附加信息，分开存零风险。
pub fn conversation_pins_file_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(conversations_dir(app)?.join("conversation-pins.json"))
}

pub fn load_conversation_pins(
    app: &AppHandle,
) -> Result<std::collections::HashMap<String, Vec<ConversationPin>>, String> {
    let path = conversation_pins_file_path(app)?;
    if !path.exists() {
        return Ok(Default::default());
    }
    let content = fs::read_to_string(&path).map_err(|e| format!("read conversation pins: {e}"))?;
    // 坏文件不该让侧栏起不来：钉子丢了最多是顺序回到时间序。
    Ok(serde_json::from_str(&content).unwrap_or_default())
}

pub fn set_conversation_pins(
    app: &AppHandle,
    group_id: &str,
    pins: Vec<ConversationPin>,
) -> Result<(), String> {
    let mut all = load_conversation_pins(app)?;
    if pins.is_empty() {
        all.remove(group_id);
    } else {
        all.insert(group_id.to_string(), pins);
    }
    let content = serde_json::to_string_pretty(&all)
        .map_err(|e| format!("serialize conversation pins: {e}"))?;
    atomic_write(
        &conversation_pins_file_path(app)?,
        &content,
        "conversation pins",
    )
}

#[cfg(test)]
mod persistence_tests {
    use super::*;

    fn conversation() -> Conversation {
        serde_json::from_value(serde_json::json!({
            "id": "conv_restart_read",
            "revision": 7,
            "title": "survives restart",
            "provider_id": "provider",
            "model": "model",
            "created_at": 1,
            "updated_at": 2,
            "messages": [{
                "id": "msg_1",
                "role": "user",
                "content": "persist me",
                "timestamp": 1
            }]
        }))
        .expect("conversation fixture")
    }

    #[test]
    fn persisted_conversation_is_readable_by_a_fresh_storage_owner() {
        let dir = tempfile::tempdir().expect("temporary conversation store");
        let path = dir.path().join("conv_restart_read.json");

        let persisted = write_conversation_file_at_path(&path, conversation())
            .expect("first owner persists conversation");
        drop(persisted);

        let (reloaded_index, healed) = super::super::index::load_index_or_scan_in_dir(dir.path())
            .expect("fresh owner rebuilds its index from disk");
        assert!(healed, "the fresh owner must discover the persisted file");
        assert_eq!(reloaded_index.conversations.len(), 1);
        assert_eq!(reloaded_index.conversations[0].id, "conv_restart_read");

        let reloaded = read_conversation_file(&path, "conv_restart_read")
            .expect("fresh owner reloads persisted conversation");
        assert_eq!(reloaded.id, "conv_restart_read");
        assert_eq!(reloaded.revision, 7);
        assert_eq!(reloaded.title, "survives restart");
        assert_eq!(reloaded.messages.len(), 1);
        assert_eq!(reloaded.messages[0].content, "persist me");
    }

    fn blank_item(archived: bool) -> ConversationListItem {
        serde_json::from_value(serde_json::json!({
            "id": "conv_blank",
            "title": "新对话",
            "preview": "",
            "provider_id": "p",
            "model": "m",
            "message_count": 0,
            "created_at": 1,
            "updated_at": 1,
            "archived": archived
        }))
        .expect("blank list item")
    }

    #[test]
    fn reusable_blank_skips_archived_index_entries() {
        assert!(super::reusable_blank_index_matches(
            &blank_item(false),
            "p",
            "m",
            None,
            None,
            None,
            None
        ));
        assert!(!super::reusable_blank_index_matches(
            &blank_item(true),
            "p",
            "m",
            None,
            None,
            None,
            None
        ));
    }
}
