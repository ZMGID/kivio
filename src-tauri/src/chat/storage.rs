use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Manager};

use super::{
    ChatAssistant, ChatAssistantIndex, ChatAssistantSnapshot, ChatProject, ChatProjectIndex,
    ChatSet, ChatSetIndex, Conversation, ConversationIndex, ConversationListItem,
};

const WRITE_RETRY_ATTEMPTS: usize = 3;

fn validate_conversation_id(id: &str) -> Result<(), String> {
    let valid = id.starts_with("conv_")
        && id.len() > "conv_".len()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if valid {
        Ok(())
    } else {
        Err(format!("Invalid conversation id: {id}"))
    }
}

fn validate_project_id(id: &str) -> Result<(), String> {
    let valid = id.starts_with("proj_")
        && id.len() > "proj_".len()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if valid {
        Ok(())
    } else {
        Err(format!("Invalid project id: {id}"))
    }
}

fn validate_assistant_id(id: &str) -> Result<(), String> {
    let valid = id.starts_with("asst_")
        && id.len() > "asst_".len()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if valid {
        Ok(())
    } else {
        Err(format!("Invalid assistant id: {id}"))
    }
}

pub(crate) fn atomic_write(path: &Path, content: &str, label: &str) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("{label} path has no parent"))?;
    fs::create_dir_all(parent).map_err(|e| format!("create {label} dir: {e}"))?;

    for attempt in 0..WRITE_RETRY_ATTEMPTS {
        let tmp_path = parent.join(format!(
            ".{}.tmp.{}",
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("conversation"),
            attempt
        ));

        // 直接 rename 覆盖:Windows/Unix 的 fs::rename 都会原子替换已存在目标。
        // 绝不"先 remove 再 rename"——那会制造"目标文件中途消失"的窗口:一旦紧接的
        // rename 失败,index.json 就没了,下次读到空索引会把其余对话文件全部孤立(数据看似丢失)。
        // 瞬时失败(锁 / 杀软占用)交给下面的外层重试循环 sleep 后重试整次写,期间旧文件始终保留。
        let write_result = fs::write(&tmp_path, content).and_then(|_| fs::rename(&tmp_path, path));

        match write_result {
            Ok(()) => return Ok(()),
            Err(e) if attempt + 1 < WRITE_RETRY_ATTEMPTS => {
                let _ = fs::remove_file(&tmp_path);
                thread::sleep(Duration::from_millis(20 * (attempt as u64 + 1)));
                if e.kind() == ErrorKind::NotFound {
                    fs::create_dir_all(parent).map_err(|e| format!("create {label} dir: {e}"))?;
                }
            }
            Err(e) => {
                let _ = fs::remove_file(&tmp_path);
                return Err(format!("write {label} file: {e}"));
            }
        }
    }

    Err(format!("write {label} file failed"))
}

fn read_conversation_file(path: &Path, id: &str) -> Result<Conversation, String> {
    let content = fs::read_to_string(path).map_err(|e| format!("读取对话文件失败（{id}）：{e}"))?;
    serde_json::from_str(&content).map_err(|e| format!("对话文件已损坏，无法加载（{id}）：{e}"))
}

fn load_conversation_list_from_files(app: &AppHandle) -> Result<Vec<ConversationListItem>, String> {
    let dir = conversations_dir(app)?;
    let entries = fs::read_dir(&dir).map_err(|e| format!("read conversations dir: {e}"))?;
    let mut conversations = Vec::new();

    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(e) => {
                eprintln!("skip unreadable conversation dir entry: {e}");
                continue;
            }
        };
        let path = entry.path();
        if path.file_name().and_then(|name| name.to_str()) == Some("index.json")
            || path.extension().and_then(|ext| ext.to_str()) != Some("json")
        {
            continue;
        }

        let id = match path.file_stem().and_then(|stem| stem.to_str()) {
            Some(id) if validate_conversation_id(id).is_ok() => id,
            _ => continue,
        };

        match read_conversation_file(&path, id) {
            Ok(conversation) => conversations.push(ConversationListItem::from(&conversation)),
            Err(e) => eprintln!("skip corrupt conversation file {id}: {e}"),
        }
    }

    Ok(conversations)
}

fn load_index_or_scan(app: &AppHandle) -> Result<ConversationIndex, String> {
    // index.json 只是缓存；conv_<id>.json 才是真相源。读取时用文件对账缓存:
    // 只要有对话文件不在索引里(索引残缺/缺失/写坏),就以文件为准重扫重建并写回修复,
    // 从根本上杜绝"残缺索引覆盖真实数据引用"导致的对话消失。
    let file_ids = conversation_file_ids(app).unwrap_or_default();
    match load_index(app) {
        Ok(index) => {
            let indexed: std::collections::HashSet<&str> =
                index.conversations.iter().map(|c| c.id.as_str()).collect();
            // 索引覆盖了每个磁盘对话文件 → 信任它(允许含多余幽灵条目,无害);
            // 缺任一文件 → 索引残缺 → 重建。
            if file_ids.iter().all(|id| indexed.contains(id.as_str())) {
                Ok(index)
            } else {
                rebuild_and_heal_index(app)
            }
        }
        Err(e) => {
            eprintln!("conversation index unavailable, rebuilding list from files: {e}");
            rebuild_and_heal_index(app)
        }
    }
}

/// 仅按文件名廉价收集磁盘上的有效对话 id(不读文件内容)。
/// `validate_conversation_id` 要求 `conv_` 前缀 → 天然排除 index/projects/assistants.json。
fn conversation_file_ids(app: &AppHandle) -> Result<Vec<String>, String> {
    let dir = conversations_dir(app)?;
    conversation_file_ids_in_dir(&dir)
}

/// 纯逻辑:扫描给定目录,收集有效对话 id(便于单测)。
fn conversation_file_ids_in_dir(dir: &Path) -> Result<Vec<String>, String> {
    let mut ids = Vec::new();
    for entry in fs::read_dir(dir).map_err(|e| format!("read conversations dir: {e}"))? {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
            if validate_conversation_id(stem).is_ok() {
                ids.push(stem.to_string());
            }
        }
    }
    Ok(ids)
}

/// 从对话文件重扫重建列表,并尽力写回修复 index.json(写失败仅告警,不影响返回)。
fn rebuild_and_heal_index(app: &AppHandle) -> Result<ConversationIndex, String> {
    let index = ConversationIndex {
        conversations: load_conversation_list_from_files(app)?,
    };
    if let Err(e) = save_index(app, &index) {
        eprintln!("heal conversation index write failed (non-fatal): {e}");
    }
    Ok(index)
}

/// 获取对话存储根目录：{app_data_dir}/conversations/
pub fn conversations_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let base = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app_data_dir unavailable: {e}"))?;
    let dir = base.join("conversations");
    if !dir.exists() {
        fs::create_dir_all(&dir).map_err(|e| format!("create conversations dir: {e}"))?;
    }
    Ok(dir)
}

/// 获取对话索引文件路径
pub fn index_file_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(conversations_dir(app)?.join("index.json"))
}

/// 获取项目索引文件路径。项目与对话同属 Chat 数据域，保存在 conversations 下便于备份/迁移。
pub fn projects_file_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(conversations_dir(app)?.join("projects.json"))
}

/// 获取助手索引文件路径。助手是 Chat 数据域的一部分，与对话一起备份/迁移。
pub fn assistants_file_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(conversations_dir(app)?.join("assistants.json"))
}

/// 获取对话文件路径
pub fn conversation_file_path(app: &AppHandle, id: &str) -> Result<PathBuf, String> {
    validate_conversation_id(id)?;
    Ok(conversations_dir(app)?.join(format!("{}.json", id)))
}

/// 获取对话附件目录
pub fn conversation_attachments_dir(app: &AppHandle, id: &str) -> Result<PathBuf, String> {
    validate_conversation_id(id)?;
    let dir = conversations_dir(app)?.join(format!("{}_attachments", id));
    if !dir.exists() {
        fs::create_dir_all(&dir).map_err(|e| format!("create attachments dir: {e}"))?;
    }
    Ok(dir)
}

/// 加载对话索引
pub fn load_index(app: &AppHandle) -> Result<ConversationIndex, String> {
    let path = index_file_path(app)?;
    if !path.exists() {
        return Ok(ConversationIndex::default());
    }

    let content = fs::read_to_string(&path).map_err(|e| format!("read index file: {e}"))?;
    serde_json::from_str(&content).map_err(|e| format!("parse index file: {e}"))
}

/// 保存对话索引
pub fn save_index(app: &AppHandle, index: &ConversationIndex) -> Result<(), String> {
    let path = index_file_path(app)?;
    let content =
        serde_json::to_string_pretty(index).map_err(|e| format!("serialize index: {e}"))?;
    atomic_write(&path, &content, "index")
}

pub fn load_project_index(app: &AppHandle) -> Result<ChatProjectIndex, String> {
    let path = projects_file_path(app)?;
    if !path.exists() {
        return Ok(ChatProjectIndex::default());
    }

    let content = fs::read_to_string(&path).map_err(|e| format!("read projects file: {e}"))?;
    let mut index: ChatProjectIndex =
        serde_json::from_str(&content).map_err(|e| format!("parse projects file: {e}"))?;
    for project in &mut index.projects {
        project.root_path = project.root_path.as_ref().and_then(|path| {
            let trimmed = path.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        });
    }
    Ok(index)
}

pub fn save_project_index(app: &AppHandle, index: &ChatProjectIndex) -> Result<(), String> {
    let path = projects_file_path(app)?;
    let content =
        serde_json::to_string_pretty(index).map_err(|e| format!("serialize projects: {e}"))?;
    atomic_write(&path, &content, "projects")
}

pub fn load_assistant_index(app: &AppHandle) -> Result<ChatAssistantIndex, String> {
    let path = assistants_file_path(app)?;
    if !path.exists() {
        // 重建后不再内置默认助手,启动为空,由用户自建。
        return Ok(ChatAssistantIndex::default());
    }

    let content = fs::read_to_string(&path).map_err(|e| format!("read assistants file: {e}"))?;
    let index: ChatAssistantIndex =
        serde_json::from_str(&content).map_err(|e| format!("parse assistants file: {e}"))?;
    Ok(index)
}

pub fn save_assistant_index(app: &AppHandle, index: &ChatAssistantIndex) -> Result<(), String> {
    let path = assistants_file_path(app)?;
    let content =
        serde_json::to_string_pretty(index).map_err(|e| format!("serialize assistants: {e}"))?;
    atomic_write(&path, &content, "assistants")
}

/// 内置专家模板（v1）：写作 / 编程 / 研究 / 数据分析。
///
/// `ChatAssistant` 没有原生工具白名单（只有 mcp_server_ids + skill_ids），所以人设主要靠
/// `system_prompt`，文件/联网/Python 等原生工具由全局 Chat 工具开关决定。这里：
/// - provider_id + model 留空 ⇒ 继承用户在 UI 选择的模型（不假设具体 provider 存在）；
/// - mcp_server_ids 留空 ⇒ 不绑定任何 MCP 服务器；
/// - skill_ids 仅引用内置文档技能（pdf/docx/xlsx/doc-coauthoring）。
pub fn builtin_assistant_definitions(now: i64) -> Vec<ChatAssistant> {
    let make = |id: &str,
                name: &str,
                icon: &str,
                color: &str,
                description: &str,
                system_prompt: &str,
                skill_ids: &[&str]| ChatAssistant {
        id: id.to_string(),
        name: name.to_string(),
        description: description.to_string(),
        icon: icon.to_string(),
        color: color.to_string(),
        source: "builtin".to_string(),
        system_prompt: system_prompt.to_string(),
        provider_id: String::new(),
        model: String::new(),
        mcp_server_ids: Vec::new(),
        skill_ids: skill_ids.iter().map(|s| s.to_string()).collect(),
        enabled: true,
        installed: true,
        archived: false,
        built_in: true,
        created_at: now,
        updated_at: now,
    };

    vec![
        make(
            "asst_builtin_writer",
            "写作助手",
            "✍️",
            "#C56646",
            "起草、改写、润色与精简文章 / 邮件 / 文案 / 报告，按你的读者与语气产出。",
            "你是一名专业的写作助手，擅长起草、改写、润色与精简各类文本：文章、报告、邮件、文案、演讲稿等。\
工作方式：动笔前先确认目标读者、用途与期望的语气和篇幅，再产出。输出要结构清晰、用词准确、避免空话套话；\
改写时保留原意并简要指出关键改动。除非用户另行指定，默认使用与用户相同的语言写作。需要长文档协作时可使用文档协作技能。",
            &["doc-coauthoring", "docx", "pdf"],
        ),
        make(
            "asst_builtin_coder",
            "编程助手",
            "💻",
            "#4F8A8B",
            "读写代码、调试、重构与解释，做最小聚焦的改动并说明改了什么、为什么。",
            "你是一名严谨的编程助手，擅长读写代码、调试、重构与解释。\
工作方式：动手前先读相关文件与上下文，做最小、聚焦的改动，并清楚说明改了什么、为什么。\
遵循项目既有的代码风格与约定；涉及命令或脚本时谨慎执行并解释其影响。给出代码时确保可运行、含必要的错误处理。\
不确定之处主动指出，绝不臆造接口或事实。",
            &[],
        ),
        make(
            "asst_builtin_researcher",
            "研究助手",
            "🔍",
            "#6A8FBD",
            "联网检索 + 阅读资料，交叉核实后给出带出处的结构化综述（只做调研，不改文件）。",
            "你是一名研究助手，负责检索、核实并综合信息，给出有出处的结论。\
工作方式：在可用时联网检索，并结合资料阅读交叉验证关键事实，明确区分事实与推测。\
输出为结构化综述：先给结论，再列论据，并附上来源链接。你只做调研与综述，不修改用户的文件。\
信息不足或来源相互冲突时如实说明，不强行下结论。",
            &[],
        ),
        make(
            "asst_builtin_data",
            "数据分析",
            "📊",
            "#7A9A57",
            "读取 PDF / Excel / Word，用 Python 做数据清洗、统计计算与可视化。",
            "你是一名数据分析助手，擅长读取并分析 PDF、Excel/CSV、Word 等文档，做数据清洗、统计计算与可视化。\
工作方式：先了解数据结构与分析目标，再用 Python（沙箱）完成处理与作图，并给出可复现的步骤与结论。\
结论要落到具体数字与图表，主动指出数据质量问题与所做的假设。可使用 pdf/docx/xlsx 文档技能读取附件。",
            &["pdf", "docx", "xlsx"],
        ),
    ]
}

/// 一次性内置专家迁移（v1）：用 `builtin_assistant_definitions` **覆盖整个**助手索引
/// （清空含用户自建的全部专家——这是用户明确选择），只留这 4 个内置专家。
///
/// 幂等性由调用方通过 `settings.builtin_assistants_seeded_v1` 标记保证；调用方必须在本函数
/// 成功后立即持久化该标记，否则下次启动会再次覆盖（连用户届时新建的专家一起抹掉）。
pub fn seed_builtin_assistants_v1(app: &AppHandle, now: i64) -> Result<(), String> {
    let index = ChatAssistantIndex {
        assistants: builtin_assistant_definitions(now),
    };
    save_assistant_index(app, &index)
}

/// 加载对话详情
pub fn load_conversation(app: &AppHandle, id: &str) -> Result<Conversation, String> {
    let path = conversation_file_path(app, id)?;
    if !path.exists() {
        return Err(format!("对话不存在：{id}"));
    }

    read_conversation_file(&path, id)
}

/// 保存对话详情
pub fn save_conversation(app: &AppHandle, conversation: &Conversation) -> Result<(), String> {
    let path = conversation_file_path(app, &conversation.id)?;

    // 保存时顺带瘦身:把内联的大图 artifact 外置到磁盘(新消息首存即生效;老对话下次保存自动迁移)。
    // 仅在确实存在这类 artifact 时才克隆,稳态下零额外开销。
    let slimmed;
    let to_save: &Conversation = if conversation
        .messages
        .iter()
        .any(super::attachments::message_has_inline_image_to_externalize)
    {
        let mut clone = conversation.clone();
        let conv_id = clone.id.clone();
        for message in clone.messages.iter_mut() {
            super::attachments::externalize_message_artifacts(app, &conv_id, message);
        }
        slimmed = clone;
        &slimmed
    } else {
        conversation
    };

    let content = serde_json::to_string_pretty(to_save)
        .map_err(|e| format!("serialize conversation: {e}"))?;
    atomic_write(&path, &content, "conversation")?;

    // 更新索引
    let mut index = load_index_or_scan(app)?;
    let list_item = ConversationListItem::from(to_save);

    if let Some(pos) = index
        .conversations
        .iter()
        .position(|c| c.id == to_save.id)
    {
        index.conversations[pos] = list_item;
    } else {
        index.conversations.insert(0, list_item);
    }

    save_index(app, &index)
}

pub fn save_conversation_without_index(
    app: &AppHandle,
    conversation: &Conversation,
) -> Result<(), String> {
    let path = conversation_file_path(app, &conversation.id)?;
    let content = serde_json::to_string_pretty(conversation)
        .map_err(|e| format!("serialize conversation: {e}"))?;
    atomic_write(&path, &content, "conversation")
}

/// 删除对话
pub fn delete_conversation(app: &AppHandle, id: &str) -> Result<(), String> {
    // 删除对话文件
    let path = conversation_file_path(app, id)?;
    if path.exists() {
        fs::remove_file(&path).map_err(|e| format!("delete conversation file: {e}"))?;
    }

    // 删除附件目录
    let attachments_dir = conversations_dir(app)?.join(format!("{}_attachments", id));
    if attachments_dir.exists() {
        fs::remove_dir_all(&attachments_dir).map_err(|e| format!("delete attachments dir: {e}"))?;
    }

    crate::native_tools::remove_sandbox_exports_for_conversation(id);

    // 更新索引
    let mut index = load_index_or_scan(app)?;
    index.conversations.retain(|c| c.id != id);
    save_index(app, &index)
}

/// 获取对话列表（分页）
pub fn get_conversations(
    app: &AppHandle,
    offset: usize,
    limit: usize,
    folder: Option<String>,
    project_id: Option<String>,
    set_id: Option<String>,
) -> Result<Vec<ConversationListItem>, String> {
    let mut index = load_index_or_scan(app)?;
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

/// 全量索引搜索：在所有对话（不止侧栏默认加载的前 N 个）的标题/预览/文件夹里做大小写
/// 不敏感子串匹配，按更新时间倒序返回前 limit 个。让侧栏搜索能找到已掉出"最近"列表的老对话。
/// 仅读 index.json（轻量元数据），不读对话正文，因此与对话总数无关地廉价。
pub fn search_conversations(
    app: &AppHandle,
    query: &str,
    limit: usize,
) -> Result<Vec<ConversationListItem>, String> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return Ok(vec![]);
    }
    let mut index = load_index_or_scan(app)?;
    index.conversations.retain(|c| {
        c.title.to_lowercase().contains(&needle)
            || c.preview.to_lowercase().contains(&needle)
            || c
                .folder
                .as_deref()
                .map(|f| f.to_lowercase().contains(&needle))
                .unwrap_or(false)
    });
    index
        .conversations
        .sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    index.conversations.truncate(limit);
    Ok(index.conversations)
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
        if item.message_count != 0 {
            continue;
        }
        if item.provider_id != provider_id || item.model != model {
            continue;
        }
        if item.folder.as_deref() != folder {
            continue;
        }
        if item.project_id.as_deref() != project_id {
            continue;
        }
        if item.set_id.as_deref() != set_id {
            continue;
        }
        if item.assistant_id.as_deref() != assistant_id {
            continue;
        }
        let conversation = match load_conversation(app, &item.id) {
            Ok(conversation) => conversation,
            Err(err) => {
                eprintln!("skip reusable blank conversation {}: {err}", item.id);
                continue;
            }
        };
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

pub fn get_projects(app: &AppHandle) -> Result<Vec<ChatProject>, String> {
    let mut project_index = load_project_index(app)?;
    let conversation_index = load_index_or_scan(app)?;
    let now = chrono::Local::now().timestamp();
    let mut changed = false;

    for folder in conversation_index
        .conversations
        .iter()
        .filter_map(|conversation| conversation.folder.as_deref())
        .map(str::trim)
        .filter(|folder| !folder.is_empty())
    {
        if project_index
            .projects
            .iter()
            .any(|project| project.name == folder)
        {
            continue;
        }
        project_index.projects.push(ChatProject {
            id: format!("proj_{}", uuid::Uuid::new_v4()),
            name: folder.to_string(),
            description: None,
            color: None,
            root_path: None,
            created_at: now,
            updated_at: now,
        });
        changed = true;
    }

    project_index.projects.sort_by(|a, b| {
        b.updated_at
            .cmp(&a.updated_at)
            .then_with(|| a.name.cmp(&b.name))
    });

    if changed {
        save_project_index(app, &project_index)?;
    }

    Ok(project_index.projects)
}

pub fn get_assistants(
    app: &AppHandle,
    include_archived: bool,
) -> Result<Vec<ChatAssistant>, String> {
    let index = load_assistant_index(app)?;
    let mut assistants = index.assistants;
    if !include_archived {
        assistants.retain(|assistant| !assistant.archived);
    }
    assistants.sort_by(|a, b| {
        b.updated_at
            .cmp(&a.updated_at)
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(assistants)
}

pub fn get_assistant(app: &AppHandle, assistant_id: &str) -> Result<ChatAssistant, String> {
    validate_assistant_id(assistant_id)?;
    load_assistant_index(app)?
        .assistants
        .into_iter()
        .find(|assistant| assistant.id == assistant_id)
        .ok_or_else(|| "助手不存在".to_string())
}

pub fn create_assistant(
    app: &AppHandle,
    mut assistant: ChatAssistant,
) -> Result<ChatAssistant, String> {
    validate_assistant_id(&assistant.id)?;
    normalize_assistant(&mut assistant)?;
    let mut index = load_assistant_index(app)?;
    if index.assistants.iter().any(|item| item.id == assistant.id) {
        return Err("助手 ID 已存在".to_string());
    }
    if index
        .assistants
        .iter()
        .any(|item| !item.archived && item.name == assistant.name)
    {
        return Err("助手名称已存在".to_string());
    }
    index.assistants.insert(0, assistant.clone());
    save_assistant_index(app, &index)?;
    Ok(assistant)
}

pub fn update_assistant(
    app: &AppHandle,
    assistant: ChatAssistant,
) -> Result<ChatAssistant, String> {
    validate_assistant_id(&assistant.id)?;
    let mut next = assistant;
    normalize_assistant(&mut next)?;
    let mut index = load_assistant_index(app)?;
    let pos = index
        .assistants
        .iter()
        .position(|item| item.id == next.id)
        .ok_or_else(|| "助手不存在".to_string())?;
    if index
        .assistants
        .iter()
        .any(|item| item.id != next.id && !item.archived && item.name == next.name)
    {
        return Err("助手名称已存在".to_string());
    }
    next.built_in = index.assistants[pos].built_in;
    next.created_at = index.assistants[pos].created_at;
    index.assistants[pos] = next.clone();
    save_assistant_index(app, &index)?;
    Ok(next)
}

pub fn duplicate_assistant(app: &AppHandle, assistant_id: &str) -> Result<ChatAssistant, String> {
    let source = get_assistant(app, assistant_id)?;
    let now = chrono::Local::now().timestamp();
    let copy = ChatAssistant {
        id: format!("asst_{}", uuid::Uuid::new_v4()),
        name: unique_assistant_copy_name(app, &source.name)?,
        built_in: false,
        archived: false,
        created_at: now,
        updated_at: now,
        ..source
    };
    create_assistant(app, copy)
}

pub fn archive_assistant(app: &AppHandle, assistant_id: &str) -> Result<(), String> {
    validate_assistant_id(assistant_id)?;
    let mut index = load_assistant_index(app)?;
    let Some(pos) = index
        .assistants
        .iter()
        .position(|assistant| assistant.id == assistant_id)
    else {
        return Err("助手不存在".to_string());
    };
    index.assistants[pos].archived = true;
    index.assistants[pos].updated_at = chrono::Local::now().timestamp();
    save_assistant_index(app, &index)
}

pub fn create_project(app: &AppHandle, mut project: ChatProject) -> Result<ChatProject, String> {
    validate_project_id(&project.id)?;
    project.name = normalize_project_name(&project.name)?;
    project.root_path = normalize_project_root_path(project.root_path)?;
    let mut index = load_project_index(app)?;
    if index.projects.iter().any(|item| item.name == project.name) {
        return Err("项目名称已存在".to_string());
    }
    index.projects.insert(0, project.clone());
    save_project_index(app, &index)?;
    Ok(project)
}

pub fn update_project(
    app: &AppHandle,
    project_id: &str,
    name: Option<String>,
    description: Option<String>,
    description_set: bool,
    color: Option<String>,
    color_set: bool,
    root_path: Option<String>,
    root_path_set: bool,
) -> Result<ChatProject, String> {
    validate_project_id(project_id)?;
    let mut project_index = load_project_index(app)?;
    let pos = project_index
        .projects
        .iter()
        .position(|project| project.id == project_id)
        .ok_or_else(|| "项目不存在".to_string())?;

    let old_name = project_index.projects[pos].name.clone();
    let new_name = match name {
        Some(name) => Some(normalize_project_name(&name)?),
        None => None,
    };
    if let Some(next_name) = new_name.as_deref() {
        if next_name != old_name
            && project_index
                .projects
                .iter()
                .any(|project| project.name == next_name)
        {
            return Err("项目名称已存在".to_string());
        }
    }

    if let Some(next_name) = new_name {
        project_index.projects[pos].name = next_name;
    }
    if description_set {
        project_index.projects[pos].description = description;
    }
    if color_set {
        project_index.projects[pos].color = color;
    }
    if root_path_set {
        project_index.projects[pos].root_path = normalize_project_root_path(root_path)?;
    }
    project_index.projects[pos].updated_at = chrono::Local::now().timestamp();
    let project = project_index.projects[pos].clone();
    save_project_index(app, &project_index)?;

    if project.name != old_name {
        move_project_conversations(app, &old_name, Some(&project.id), Some(&project.name))?;
    }

    Ok(project)
}

pub fn delete_project(app: &AppHandle, project_id: &str) -> Result<(), String> {
    validate_project_id(project_id)?;
    let mut project_index = load_project_index(app)?;
    let Some(pos) = project_index
        .projects
        .iter()
        .position(|project| project.id == project_id)
    else {
        return Err("项目不存在".to_string());
    };
    let project = project_index.projects.remove(pos);
    save_project_index(app, &project_index)?;
    move_project_conversations(app, &project.name, Some(&project.id), None)
}

fn normalize_project_name(name: &str) -> Result<String, String> {
    let normalized = name.trim();
    if normalized.is_empty() {
        return Err("项目名称不能为空".to_string());
    }
    if normalized.chars().count() > 80 {
        return Err("项目名称不能超过 80 个字符".to_string());
    }
    Ok(normalized.to_string())
}

fn normalize_project_root_path(root_path: Option<String>) -> Result<Option<String>, String> {
    let Some(root_path) = root_path else {
        return Ok(None);
    };
    let trimmed = root_path.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let expanded = expand_home_prefix(trimmed)?;
    let path = Path::new(&expanded);
    if !path.is_absolute() {
        return Err("项目文件夹必须是绝对路径。".to_string());
    }
    if !path.is_dir() {
        return Err("项目文件夹不存在或不是文件夹。".to_string());
    }
    fs::canonicalize(path)
        .map(|path| Some(path.to_string_lossy().to_string()))
        .map_err(|err| format!("解析项目文件夹失败：{err}"))
}

fn expand_home_prefix(raw_path: &str) -> Result<String, String> {
    if raw_path == "~" {
        return user_home_dir().map(|path| path.to_string_lossy().to_string());
    }
    if let Some(rest) = raw_path.strip_prefix("~/") {
        return user_home_dir().map(|home| home.join(rest).to_string_lossy().to_string());
    }
    #[cfg(target_os = "windows")]
    if let Some(rest) = raw_path.strip_prefix("~\\") {
        return user_home_dir().map(|home| home.join(rest).to_string_lossy().to_string());
    }
    Ok(raw_path.to_string())
}

fn user_home_dir() -> Result<PathBuf, String> {
    #[cfg(target_os = "windows")]
    {
        std::env::var("USERPROFILE")
            .map(PathBuf::from)
            .map_err(|_| "USERPROFILE is not set".to_string())
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::env::var("HOME")
            .map(PathBuf::from)
            .map_err(|_| "HOME is not set".to_string())
    }
}

pub fn find_project_by_id(app: &AppHandle, project_id: &str) -> Result<ChatProject, String> {
    validate_project_id(project_id)?;
    load_project_index(app)?
        .projects
        .into_iter()
        .find(|project| project.id == project_id)
        .ok_or_else(|| "项目不存在".to_string())
}

pub fn find_project_by_name(app: &AppHandle, name: &str) -> Result<Option<ChatProject>, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    Ok(load_project_index(app)?
        .projects
        .into_iter()
        .find(|project| project.name == trimmed))
}

// ===== Chat 集(Set) 存储：照搬 project 模式，去掉 root_path/folder 迁移，加 system_prompt/默认助手 =====

fn validate_set_id(id: &str) -> Result<(), String> {
    let valid = id.starts_with("set_")
        && id.len() > "set_".len()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if valid {
        Ok(())
    } else {
        Err(format!("Invalid set id: {id}"))
    }
}

fn normalize_set_name(name: &str) -> Result<String, String> {
    let normalized = name.trim();
    if normalized.is_empty() {
        return Err("集名称不能为空".to_string());
    }
    Ok(normalized.chars().take(80).collect())
}

pub fn sets_file_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(conversations_dir(app)?.join("sets.json"))
}

pub fn load_set_index(app: &AppHandle) -> Result<ChatSetIndex, String> {
    let path = sets_file_path(app)?;
    if !path.exists() {
        return Ok(ChatSetIndex::default());
    }
    let content = fs::read_to_string(&path).map_err(|e| format!("read sets file: {e}"))?;
    serde_json::from_str(&content).map_err(|e| format!("parse sets file: {e}"))
}

pub fn save_set_index(app: &AppHandle, index: &ChatSetIndex) -> Result<(), String> {
    let path = sets_file_path(app)?;
    let content =
        serde_json::to_string_pretty(index).map_err(|e| format!("serialize sets: {e}"))?;
    atomic_write(&path, &content, "sets")
}

pub fn get_sets(app: &AppHandle) -> Result<Vec<ChatSet>, String> {
    Ok(load_set_index(app)?.sets)
}

pub fn find_set_by_id(app: &AppHandle, set_id: &str) -> Result<ChatSet, String> {
    validate_set_id(set_id)?;
    load_set_index(app)?
        .sets
        .into_iter()
        .find(|set| set.id == set_id)
        .ok_or_else(|| "集不存在".to_string())
}

pub fn create_set(app: &AppHandle, mut set: ChatSet) -> Result<ChatSet, String> {
    validate_set_id(&set.id)?;
    set.name = normalize_set_name(&set.name)?;
    let mut index = load_set_index(app)?;
    if index.sets.iter().any(|item| item.name == set.name) {
        return Err("集名称已存在".to_string());
    }
    index.sets.insert(0, set.clone());
    save_set_index(app, &index)?;
    Ok(set)
}

#[allow(clippy::too_many_arguments)]
pub fn update_set(
    app: &AppHandle,
    set_id: &str,
    name: Option<String>,
    system_prompt: Option<String>,
    system_prompt_set: bool,
    default_assistant_id: Option<String>,
    default_assistant_id_set: bool,
    color: Option<String>,
    color_set: bool,
) -> Result<ChatSet, String> {
    validate_set_id(set_id)?;
    let mut index = load_set_index(app)?;
    let pos = index
        .sets
        .iter()
        .position(|set| set.id == set_id)
        .ok_or_else(|| "集不存在".to_string())?;

    let old_name = index.sets[pos].name.clone();
    if let Some(name) = name {
        let next_name = normalize_set_name(&name)?;
        if next_name != old_name && index.sets.iter().any(|set| set.name == next_name) {
            return Err("集名称已存在".to_string());
        }
        index.sets[pos].name = next_name;
    }
    if system_prompt_set {
        index.sets[pos].system_prompt = system_prompt.unwrap_or_default();
    }
    if default_assistant_id_set {
        index.sets[pos].default_assistant_id = default_assistant_id.and_then(|id| {
            let trimmed = id.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        });
    }
    if color_set {
        index.sets[pos].color = color;
    }
    index.sets[pos].updated_at = chrono::Local::now().timestamp();
    let set = index.sets[pos].clone();
    save_set_index(app, &index)?;
    Ok(set)
}

pub fn delete_set(app: &AppHandle, set_id: &str) -> Result<(), String> {
    validate_set_id(set_id)?;
    let mut index = load_set_index(app)?;
    let Some(pos) = index.sets.iter().position(|set| set.id == set_id) else {
        return Err("集不存在".to_string());
    };
    index.sets.remove(pos);
    save_set_index(app, &index)?;
    clear_set_from_conversations(app, set_id)
}

/// 删除集后，把名下对话的 set_id 清空（对话回到散对话、不丢）。仿 move_project_conversations。
fn clear_set_from_conversations(app: &AppHandle, set_id: &str) -> Result<(), String> {
    let mut index = load_index_or_scan(app)?;
    let mut changed = false;
    for item in &mut index.conversations {
        if item.set_id.as_deref() != Some(set_id) {
            continue;
        }
        let mut conversation = load_conversation(app, &item.id)?;
        conversation.set_id = None;
        conversation.updated_at = chrono::Local::now().timestamp();
        save_conversation_without_index(app, &conversation)?;
        *item = ConversationListItem::from(&conversation);
        changed = true;
    }
    if changed {
        save_index(app, &index)?;
    }
    Ok(())
}

pub fn resolve_conversation_project(
    app: &AppHandle,
    conversation: &Conversation,
) -> Result<Option<ChatProject>, String> {
    if let Some(project_id) = conversation
        .project_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
    {
        return find_project_by_id(app, project_id).map(Some);
    }
    if let Some(folder) = conversation
        .folder
        .as_deref()
        .map(str::trim)
        .filter(|folder| !folder.is_empty())
    {
        return find_project_by_name(app, folder);
    }
    Ok(None)
}

pub fn assistant_snapshot(
    app: &AppHandle,
    assistant_id: &str,
) -> Result<ChatAssistantSnapshot, String> {
    let assistant = get_assistant(app, assistant_id)?;
    if assistant.archived || !assistant.enabled {
        return Err("助手不可用".to_string());
    }
    Ok(ChatAssistantSnapshot::from(&assistant))
}

fn normalize_assistant(assistant: &mut ChatAssistant) -> Result<(), String> {
    assistant.name = assistant.name.trim().to_string();
    if assistant.name.is_empty() {
        return Err("助手名称不能为空".to_string());
    }
    if assistant.name.chars().count() > 64 {
        return Err("助手名称不能超过 64 个字符".to_string());
    }
    assistant.description = assistant.description.trim().to_string();
    if assistant.description.chars().count() > 240 {
        return Err("助手描述不能超过 240 个字符".to_string());
    }
    assistant.icon = assistant.icon.trim().chars().take(8).collect();
    assistant.color = assistant.color.trim().chars().take(32).collect();
    assistant.source = normalize_assistant_source(&assistant.source, assistant.built_in);
    assistant.system_prompt = assistant.system_prompt.trim().to_string();
    assistant.provider_id = assistant.provider_id.trim().to_string();
    assistant.model = assistant.model.trim().to_string();
    assistant.mcp_server_ids = normalize_string_list(&assistant.mcp_server_ids, 64, 200);
    assistant.skill_ids = normalize_string_list(&assistant.skill_ids, 64, 200);
    Ok(())
}

fn normalize_assistant_source(source: &str, built_in: bool) -> String {
    match source.trim() {
        "builtin" | "user" | "imported" => source.trim().to_string(),
        _ if built_in => "builtin".to_string(),
        _ => "user".to_string(),
    }
}

fn normalize_string_list(values: &[String], limit: usize, max_chars: usize) -> Vec<String> {
    let mut out = Vec::new();
    for value in values {
        let item: String = value.trim().chars().take(max_chars).collect();
        if item.is_empty() || out.iter().any(|existing| existing == &item) {
            continue;
        }
        out.push(item);
        if out.len() >= limit {
            break;
        }
    }
    out
}

fn unique_assistant_copy_name(app: &AppHandle, base_name: &str) -> Result<String, String> {
    let index = load_assistant_index(app)?;
    let base = format!("{base_name} 副本");
    if !index
        .assistants
        .iter()
        .any(|assistant| !assistant.archived && assistant.name == base)
    {
        return Ok(base);
    }
    for i in 2..100 {
        let candidate = format!("{base} {i}");
        if !index
            .assistants
            .iter()
            .any(|assistant| !assistant.archived && assistant.name == candidate)
        {
            return Ok(candidate);
        }
    }
    Ok(format!("{base} {}", chrono::Local::now().timestamp()))
}

fn move_project_conversations(
    app: &AppHandle,
    old_name: &str,
    old_project_id: Option<&str>,
    next_name: Option<&str>,
) -> Result<(), String> {
    let mut index = load_index_or_scan(app)?;
    let mut changed = false;
    for item in &mut index.conversations {
        let belongs_to_project = item.folder.as_deref() == Some(old_name)
            || old_project_id
                .map(|project_id| item.project_id.as_deref() == Some(project_id))
                .unwrap_or(false);
        if !belongs_to_project {
            continue;
        }
        let mut conversation = load_conversation(app, &item.id)?;
        conversation.folder = next_name.map(str::to_string);
        if next_name.is_none() {
            conversation.project_id = None;
        }
        conversation.updated_at = chrono::Local::now().timestamp();
        save_conversation_without_index(app, &conversation)?;
        *item = ConversationListItem::from(&conversation);
        changed = true;
    }
    if changed {
        save_index(app, &index)?;
    }
    Ok(())
}

#[cfg(test)]
mod builtin_assistant_tests {
    use super::*;

    #[test]
    fn set_id_validation_accepts_prefixed_ids_rejects_others() {
        assert!(validate_set_id("set_abc-123").is_ok());
        assert!(validate_set_id("set_").is_err()); // 仅前缀无内容
        assert!(validate_set_id("proj_abc").is_err()); // 错误前缀
        assert!(validate_set_id("set_a/b").is_err()); // 非法字符
        assert!(validate_set_id("abc").is_err());
    }

    #[test]
    fn set_name_normalization_trims_caps_and_rejects_empty() {
        assert_eq!(normalize_set_name("  写作集  ").unwrap(), "写作集");
        assert!(normalize_set_name("   ").is_err());
        let long: String = "x".repeat(200);
        assert_eq!(normalize_set_name(&long).unwrap().chars().count(), 80);
    }

    #[test]
    fn builtin_assistants_are_four_valid_built_in_personas() {
        let defs = builtin_assistant_definitions(1_700_000_000);
        assert_eq!(defs.len(), 4, "expected exactly 4 built-in assistants");

        let mut ids: Vec<&str> = defs.iter().map(|d| d.id.as_str()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), defs.len(), "built-in assistant ids must be unique");

        for d in &defs {
            // ids must satisfy validate_assistant_id (asst_ prefix + safe chars).
            assert!(d.id.starts_with("asst_") && d.id.len() > "asst_".len(), "{}", d.id);
            assert!(d.built_in, "{} must be built_in", d.id);
            assert_eq!(d.source, "builtin", "{}", d.id);
            assert!(d.enabled && d.installed && !d.archived, "{}", d.id);
            // Inherit the user's selected model — never pin a provider/model.
            assert!(d.provider_id.is_empty() && d.model.is_empty(), "{}", d.id);
            // Honor normalize_assistant constraints so a later edit won't reject them.
            assert!(!d.name.trim().is_empty() && d.name.chars().count() <= 64, "{}", d.id);
            assert!(d.description.chars().count() <= 240, "{}", d.id);
            assert!(d.icon.chars().count() <= 8, "{}", d.id);
            assert!(!d.system_prompt.trim().is_empty(), "{}", d.id);
        }
    }

    #[test]
    fn data_assistant_whitelists_document_skills() {
        let defs = builtin_assistant_definitions(1_700_000_000);
        let data = defs.iter().find(|d| d.id == "asst_builtin_data").unwrap();
        for skill in ["pdf", "docx", "xlsx"] {
            assert!(data.skill_ids.iter().any(|s| s == skill), "missing skill {skill}");
        }
        // Researcher/coder need no document skills.
        let coder = defs.iter().find(|d| d.id == "asst_builtin_coder").unwrap();
        assert!(coder.skill_ids.is_empty());
    }
}

#[cfg(test)]
mod index_self_heal_tests {
    use super::*;
    use std::fs;

    fn temp_dir() -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("kivio-storage-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn atomic_write_overwrites_existing_file() {
        let dir = temp_dir();
        let path = dir.join("index.json");
        atomic_write(&path, "AAA", "test").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "AAA");
        // 覆盖已存在文件应成功(不再"先删后 rename");目标文件始终有内容。
        atomic_write(&path, "BBBB", "test").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "BBBB");
        assert!(path.exists());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn conversation_file_ids_in_dir_only_collects_valid_conv_files() {
        let dir = temp_dir();
        // 有效对话文件
        fs::write(dir.join("conv_aaa.json"), "{}").unwrap();
        fs::write(dir.join("conv_bbb-1.json"), "{}").unwrap();
        // 应被排除:缓存/索引文件、非 json、非 conv_ 前缀(无效 id)
        fs::write(dir.join("index.json"), "{}").unwrap();
        fs::write(dir.join("projects.json"), "{}").unwrap();
        fs::write(dir.join("assistants.json"), "{}").unwrap();
        fs::write(dir.join("notes.txt"), "x").unwrap();
        fs::write(dir.join("random.json"), "{}").unwrap();

        let mut ids = conversation_file_ids_in_dir(&dir).unwrap();
        ids.sort();
        assert_eq!(ids, vec!["conv_aaa".to_string(), "conv_bbb-1".to_string()]);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn covers_all_logic_detects_missing_conversation_files() {
        // 模拟 load_index_or_scan 的核心判定:file_ids ⊆ indexed 才信任索引。
        let indexed: std::collections::HashSet<&str> = ["conv_a", "conv_b"].into_iter().collect();
        // 索引覆盖全部文件(还多一个幽灵条目 conv_b)→ 信任
        let files_covered = ["conv_a"];
        assert!(files_covered.iter().all(|id| indexed.contains(*id)));
        // 有文件(conv_c)不在索引 → 需重建
        let files_diverged = ["conv_a", "conv_c"];
        assert!(!files_diverged.iter().all(|id| indexed.contains(*id)));
    }
}
