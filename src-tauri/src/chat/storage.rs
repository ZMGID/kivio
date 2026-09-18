use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{BufReader, ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Manager};

use serde::Deserialize;

use super::{
    AdditionalDirectory, ChatAssistant, ChatAssistantIndex, ChatAssistantSnapshot, ChatProject,
    ChatProjectIndex, ChatSet, ChatSetIndex, Conversation, ConversationIndex, ConversationListItem,
    ConversationPin, MAX_ADDITIONAL_DIRECTORIES,
};

mod search;
#[cfg(test)]
use search::{
    compare_library_items, conversation_raw_might_contain, first_message_match,
    make_search_snippet, messages_match, rank_conversations_for_search,
};
pub use search::{
    query_conversations, search_conversations, ConversationLibraryPage, ConversationLibraryQuery,
};

mod assistants;
mod conversations;
mod index;
mod migration;
mod projects;
mod sets;

#[allow(unused_imports)]
pub(crate) use assistants::merge_builtin_definitions;
pub use assistants::{
    archive_assistant, assistant_snapshot, builtin_assistant_definitions, create_assistant,
    duplicate_assistant, get_assistant, get_assistants, load_assistant_index,
    merge_builtin_assistants_v2, merge_builtin_assistants_v3, save_assistant_index,
    seed_builtin_assistants_v1, update_assistant,
};
pub use conversations::{
    conversation_pins_file_path, find_reusable_blank_conversation, get_conversations,
    load_conversation, load_conversation_pins, set_conversation_pins,
};
pub(crate) use conversations::{
    delete_conversation, read_conversation_file, restart_goal_candidates, write_conversation_file,
    RestartGoal,
};
pub use index::load_index;
pub(crate) use index::{
    conversation_index_needs_persist, load_index_or_scan, persist_healed_conversation_index,
    save_index,
};
pub(crate) use migration::rewrite_conversation_artifact_paths;
pub use migration::{
    migrate_ordinary_conversation_workspaces, resolve_conversation_project,
    resolve_conversation_working_directory,
};
pub use projects::{
    create_project, create_project_with_options, delete_project, find_project_by_id,
    find_project_by_name, get_projects, load_project_index, normalize_additional_directories,
    reorder_projects, reorder_sets, save_project_index, update_project,
};
pub use sets::{
    create_set, delete_set, find_set_by_id, get_sets, live_set_system_prompt, load_set_index,
    save_set_index, sets_file_path, update_set,
};

#[cfg(test)]
use assistants::{assistant_is_available, canonicalize_cua_mcp_server_ids};
#[cfg(test)]
use conversations::{
    read_restart_goal, remove_conversation_side_artifacts, restart_goal_candidates_in_dir,
};
#[cfg(test)]
use index::{
    conversation_file_ids_in_dir, index_covers_files, load_index_in_dir, load_index_or_scan_in_dir,
};
#[cfg(test)]
use migration::has_non_empty_value;
#[cfg(test)]
use projects::reorder_by_ids;
#[cfg(test)]
use sets::{normalize_set_name, validate_set_id};
const WRITE_RETRY_ATTEMPTS: usize = 3;

#[cfg(test)]
thread_local! {
    static FAIL_ATOMIC_REPLACE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
fn fail_atomic_replace_for_current_test(enabled: bool) {
    FAIL_ATOMIC_REPLACE.set(enabled);
}

#[cfg(test)]
#[path = "storage_restart_tests.rs"]
mod restart_goal_tests;

fn temporary_write_path(path: &Path) -> PathBuf {
    path.parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!(
            ".{}.tmp.{}",
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("conversation"),
            uuid::Uuid::new_v4()
        ))
}

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
        let tmp_path = temporary_write_path(path);

        // 直接 rename 覆盖:Windows/Unix 的 fs::rename 都会原子替换已存在目标。
        // 绝不"先 remove 再 rename"——那会制造"目标文件中途消失"的窗口:一旦紧接的
        // rename 失败,index.json 就没了,下次读到空索引会把其余对话文件全部孤立(数据看似丢失)。
        // 瞬时失败(锁 / 杀软占用)交给下面的外层重试循环 sleep 后重试整次写,期间旧文件始终保留。
        //
        // rename 之前必须 sync_all():否则数据还在页缓存里、rename 的元数据却可能先落盘,
        // 断电后 conv_x.json 会变成 0 字节或被截断——load_conversation 硬报错,而列表扫描
        // 又会静默跳过它,用户看到的就是"这条对话没了"。
        let write_result = (|| {
            let mut file = fs::File::create(&tmp_path)?;
            file.write_all(content.as_bytes())?;
            file.sync_all()?;
            drop(file);
            #[cfg(test)]
            if FAIL_ATOMIC_REPLACE.get() {
                return Err(std::io::Error::other("injected atomic replace failure"));
            }
            fs::rename(&tmp_path, path)
        })();

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

/// 中断草稿日志路径(`{id}.draft.jsonl`,见 `chat::draft_journal`)。
/// 扩展名刻意不是 `.json`:`conversation_file_ids_in_dir` 的扫描不会把它当会话文件。
pub(crate) fn draft_journal_path(app: &AppHandle, id: &str) -> Result<PathBuf, String> {
    validate_conversation_id(id)?;
    Ok(conversations_dir(app)?.join(format!("{id}.draft.jsonl")))
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

#[cfg(test)]
mod conversation_workspace_tests {
    use super::*;

    #[test]
    fn canonicalizes_and_deduplicates_legacy_cua_mcp_ids() {
        let mut ids = vec![
            "plugin-cua-driver".to_string(),
            "computer-control-cua-driver".to_string(),
            "other".to_string(),
        ];

        canonicalize_cua_mcp_server_ids(&mut ids);

        assert_eq!(
            ids,
            vec![
                "computer-control-cua-driver".to_string(),
                "other".to_string(),
            ]
        );
    }

    #[test]
    fn messages_match_scans_content_and_reasoning() {
        let conv: Conversation = serde_json::from_value(serde_json::json!({
            "id": "conv_s", "title": "t", "provider_id": "p", "model": "m",
            "created_at": 1, "updated_at": 1,
            "messages": [
                {"id": "m1", "role": "user", "content": "帮我看看知识库配置", "timestamp": 1},
                {"id": "m2", "role": "assistant", "content": "好的", "reasoning": "需要检查 WASM 加载", "timestamp": 2}
            ]
        }))
        .expect("conversation");
        assert!(messages_match(&conv, "知识库")); // content，大小写不敏感
        assert!(messages_match(&conv, "wasm 加载")); // reasoning
        assert!(!messages_match(&conv, "不存在的词"));
    }

    fn artifact(path: &Path) -> serde_json::Value {
        serde_json::json!({
            "name": "report.txt",
            "mime_type": "text/plain",
            "data_url": "",
            "path": path.to_string_lossy()
        })
    }

    #[test]
    fn rewrites_message_and_tool_call_artifact_paths() {
        let source = PathBuf::from("C:/old/conv_test");
        let target = PathBuf::from("D:/new/conv_test");
        let outside = PathBuf::from("C:/Desktop/keep.txt");
        let direct = source.join("direct.txt");
        let nested = source.join("nested/tool.txt");
        let mut conversation: Conversation = serde_json::from_value(serde_json::json!({
            "id": "conv_test",
            "title": "test",
            "provider_id": "provider",
            "model": "model",
            "created_at": 1,
            "updated_at": 1,
            "messages": [{
                "id": "msg_1",
                "role": "assistant",
                "content": "done",
                "timestamp": 1,
                "artifacts": [artifact(&direct), artifact(&outside)],
                "tool_calls": [{
                    "id": "tool_1",
                    "name": "write",
                    "status": "success",
                    "artifacts": [artifact(&nested)]
                }]
            }]
        }))
        .expect("conversation");

        assert!(rewrite_conversation_artifact_paths(
            &mut conversation,
            &[(source, target.clone())]
        ));
        assert_eq!(
            conversation.messages[0].artifacts[0].path.as_deref(),
            Some(target.join("direct.txt").to_string_lossy().as_ref())
        );
        assert_eq!(
            conversation.messages[0].artifacts[1].path.as_deref(),
            Some(outside.to_string_lossy().as_ref())
        );
        assert_eq!(
            conversation.messages[0].tool_calls[0].artifacts[0]
                .path
                .as_deref(),
            Some(target.join("nested/tool.txt").to_string_lossy().as_ref())
        );
    }

    #[test]
    fn explicit_project_id_is_treated_as_project_binding() {
        assert!(has_non_empty_value(Some("proj_missing")));
        assert!(!has_non_empty_value(Some("  ")));
        assert!(!has_non_empty_value(None));
    }

    #[test]
    fn make_search_snippet_centers_on_match_and_stays_char_safe() {
        // 长正文 + CJK：窗口落在字符边界，前后省略号齐全，命中词仍在片段里。
        let prefix: String = "前".repeat(80);
        let suffix: String = "后".repeat(80);
        let text = format!("{prefix}关键命中词在这里{suffix}");
        let snippet = make_search_snippet(&text, "命中词");
        assert!(snippet.starts_with('…'), "snippet={snippet}");
        assert!(snippet.ends_with('…'), "snippet={snippet}");
        assert!(snippet.contains("命中词"), "snippet={snippet}");
        // 不能 panic / 产出半个码点（floor/ceil_char_boundary 兜住）
        assert!(snippet.is_char_boundary(0));
        assert!(snippet.is_char_boundary(snippet.len()));
    }

    #[test]
    fn make_search_snippet_collapses_whitespace_and_truncates_misses() {
        let messy = "line1\n\n  line2\t\tline3";
        let hit = make_search_snippet(messy, "line2");
        assert!(!hit.contains('\n'), "snippet={hit}");
        assert!(!hit.contains('\t'), "snippet={hit}");
        assert!(hit.contains("line2"), "snippet={hit}");

        let long_miss = "甲".repeat(200);
        let miss = make_search_snippet(&long_miss, "不存在");
        // 没命中时走 truncate_chars(140)——正文截到 140 字再拼 "..."
        assert!(
            miss.chars().count() <= 143,
            "miss len={}",
            miss.chars().count()
        );
        assert!(miss.ends_with("..."), "miss={miss}");
    }

    #[test]
    fn first_message_match_returns_field_message_id_and_snippet() {
        let conv: Conversation = serde_json::from_value(serde_json::json!({
            "id": "conv_s", "title": "t", "provider_id": "p", "model": "m",
            "created_at": 1, "updated_at": 1,
            "messages": [
                {"id": "m1", "role": "user", "content": "普通开场", "timestamp": 1},
                {
                    "id": "m2",
                    "role": "assistant",
                    "content": "前面一长段铺垫文字用来撑开窗口，然后出现关键词知识库配置，后面继续补上下文。",
                    "reasoning": "需要检查 WASM 加载",
                    "timestamp": 2
                }
            ]
        }))
        .expect("conversation");

        let (field, message_id, snippet) =
            first_message_match(&conv, "知识库").expect("content hit");
        assert_eq!(field, "content");
        assert_eq!(message_id, "m2");
        assert!(snippet.contains("知识库"), "snippet={snippet}");

        let (field, message_id, snippet) =
            first_message_match(&conv, "wasm 加载").expect("reasoning hit");
        assert_eq!(field, "reasoning");
        assert_eq!(message_id, "m2");
        assert!(
            snippet.contains("WASM") || snippet.to_lowercase().contains("wasm"),
            "snippet={snippet}"
        );

        // 同消息 content 与 reasoning 都命中时，content 优先（决定跳转字段）
        let both: Conversation = serde_json::from_value(serde_json::json!({
            "id": "conv_s", "title": "t", "provider_id": "p", "model": "m",
            "created_at": 1, "updated_at": 1,
            "messages": [{
                "id": "m9",
                "role": "assistant",
                "content": "正文也写了 sandbox",
                "reasoning": "reasoning 里也有 sandbox",
                "timestamp": 1
            }]
        }))
        .expect("conversation");
        let (field, message_id, _) = first_message_match(&both, "sandbox").expect("both hit");
        assert_eq!(field, "content");
        assert_eq!(message_id, "m9");
    }

    #[test]
    fn first_message_match_prefers_earlier_message() {
        let conv: Conversation = serde_json::from_value(serde_json::json!({
            "id": "conv_s", "title": "t", "provider_id": "p", "model": "m",
            "created_at": 1, "updated_at": 1,
            "messages": [
                {"id": "m1", "role": "user", "content": "第一次提到 CDN77", "timestamp": 1},
                {"id": "m2", "role": "assistant", "content": "又一次 CDN77", "timestamp": 2}
            ]
        }))
        .expect("conversation");
        let (_, message_id, _) = first_message_match(&conv, "cdn77").expect("hit");
        assert_eq!(
            message_id, "m1",
            "global search jump must land on the first hit"
        );
    }

    fn list_item(partial: serde_json::Value) -> ConversationListItem {
        let mut base = serde_json::json!({
            "id": "c",
            "title": "t",
            "preview": "",
            "provider_id": "p",
            "model": "m",
            "message_count": 1,
            "created_at": 1,
            "updated_at": 1,
            "pinned": false,
        });
        if let (Some(base_obj), Some(partial_obj)) = (base.as_object_mut(), partial.as_object()) {
            for (k, v) in partial_obj {
                base_obj.insert(k.clone(), v.clone());
            }
        }
        serde_json::from_value(base).expect("list item")
    }

    #[test]
    fn compare_library_items_pins_first_then_sorts_by_key() {
        let pinned = list_item(serde_json::json!({
            "id": "pinned",
            "pinned": true,
            "updated_at": 10,
            "title": "b"
        }));
        let recent = list_item(serde_json::json!({
            "id": "recent",
            "updated_at": 100,
            "title": "a"
        }));
        let older = list_item(serde_json::json!({
            "id": "older",
            "updated_at": 50,
            "title": "c",
            "message_count": 9,
            "created_at": 9
        }));

        // pinned always wins regardless of updated_at
        assert_eq!(
            compare_library_items(&pinned, &recent, "updated", false),
            std::cmp::Ordering::Less
        );

        // default: updated desc
        assert_eq!(
            compare_library_items(&recent, &older, "updated", false),
            std::cmp::Ordering::Less
        );
        // updated asc
        assert_eq!(
            compare_library_items(&recent, &older, "updated", true),
            std::cmp::Ordering::Greater
        );

        // title is case-insensitive
        let upper = list_item(serde_json::json!({ "id": "U", "title": "Banana" }));
        let lower = list_item(serde_json::json!({ "id": "L", "title": "apple" }));
        assert_eq!(
            compare_library_items(&lower, &upper, "title", true),
            std::cmp::Ordering::Less
        );

        // messages desc
        assert_eq!(
            compare_library_items(&older, &recent, "messages", false),
            std::cmp::Ordering::Less
        );

        // created desc
        assert_eq!(
            compare_library_items(&older, &recent, "created", false),
            std::cmp::Ordering::Less
        );
    }
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
    fn builtin_assistants_are_valid_built_in_personas() {
        let defs = builtin_assistant_definitions(1_700_000_000);
        assert_eq!(defs.len(), 13, "expected exactly 13 built-in assistants");

        let mut ids: Vec<&str> = defs.iter().map(|d| d.id.as_str()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(
            ids.len(),
            defs.len(),
            "built-in assistant ids must be unique"
        );

        for d in &defs {
            // ids must satisfy validate_assistant_id (asst_ prefix + safe chars).
            assert!(
                d.id.starts_with("asst_") && d.id.len() > "asst_".len(),
                "{}",
                d.id
            );
            assert!(d.built_in, "{} must be built_in", d.id);
            assert_eq!(d.source, "builtin", "{}", d.id);
            // 策展式：内置默认启用但未加入应用（installed=false），用户手动添加后才可用。
            assert!(d.enabled && !d.installed && !d.archived, "{}", d.id);
            // Inherit the user's selected model — never pin a provider/model.
            assert!(d.provider_id.is_empty() && d.model.is_empty(), "{}", d.id);
            // Honor normalize_assistant constraints so a later edit won't reject them.
            assert!(
                !d.name.trim().is_empty() && d.name.chars().count() <= 64,
                "{}",
                d.id
            );
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
            assert!(
                data.skill_ids.iter().any(|s| s == skill),
                "missing skill {skill}"
            );
        }
        // v2/v3 新增的专家在册，且 id 唯一（数量断言在上一个测试）。
        for id in [
            "asst_builtin_frontend",
            "asst_builtin_translator",
            "asst_builtin_docsmith",
            "asst_builtin_pm",
            "asst_builtin_legal",
            "asst_builtin_finance",
            "asst_builtin_teacher",
            "asst_builtin_reviewer",
            "asst_builtin_career",
        ] {
            assert!(defs.iter().any(|d| d.id == id), "missing {id}");
        }
        // 每个专家都拼接了去 AI 味文风块。
        for d in &defs {
            assert!(
                d.system_prompt.contains("像具体的人写的"),
                "{} missing no-AI-flavor style block",
                d.id
            );
        }
        let legal = defs.iter().find(|d| d.id == "asst_builtin_legal").unwrap();
        assert!(
            legal.system_prompt.contains("不是律师"),
            "legal persona must disclaim it is not a lawyer"
        );
        let finance = defs
            .iter()
            .find(|d| d.id == "asst_builtin_finance")
            .unwrap();
        assert!(
            finance.system_prompt.contains("不是持牌顾问"),
            "finance persona must disclaim it is not a licensed advisor"
        );
    }

    #[test]
    fn merge_v2_updates_builtins_and_preserves_user_assistants() {
        let defs = builtin_assistant_definitions(1_700_000_000);
        // 老装现状：一个旧版内置（同 id、旧 prompt）+ 一个用户自建。
        let mut old_writer = defs
            .iter()
            .find(|d| d.id == "asst_builtin_writer")
            .unwrap()
            .clone();
        old_writer.system_prompt = "旧版写作 prompt".to_string();
        let mut user = defs[0].clone();
        user.id = "asst_user_custom".to_string();
        user.built_in = false;
        user.source = "user".to_string();

        let merged = merge_builtin_definitions(
            vec![old_writer, user],
            builtin_assistant_definitions(1_700_000_000),
        );

        // 用户自建保留。
        assert!(
            merged
                .iter()
                .any(|a| a.id == "asst_user_custom" && !a.built_in),
            "user assistant must be preserved"
        );
        // 旧内置被新版覆盖（新版含文风块）。
        let w = merged
            .iter()
            .find(|a| a.id == "asst_builtin_writer")
            .unwrap();
        assert!(w.system_prompt.contains("像具体的人写的"));
        // 新增内置补齐。
        assert!(merged.iter().any(|a| a.id == "asst_builtin_translator"));
        assert!(merged.iter().any(|a| a.id == "asst_builtin_pm"));
        // 13 内置 + 1 用户，无重复。
        assert_eq!(merged.len(), 14);
        assert_eq!(merged.iter().filter(|a| a.built_in).count(), 13);
    }

    #[test]
    fn legacy_disabled_assistant_remains_available_until_archived() {
        let mut assistant = builtin_assistant_definitions(1_700_000_000)
            .into_iter()
            .next()
            .unwrap();
        assistant.enabled = false;
        assert!(assistant_is_available(&assistant));

        assistant.archived = true;
        assert!(!assistant_is_available(&assistant));
    }
}

#[cfg(test)]
mod index_self_heal_tests {
    use super::*;
    use std::fs;
    use std::sync::{Arc, Mutex};
    use std::thread;

    fn temp_dir() -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("kivio-storage-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn list_item(id: &str, revision: Option<u64>) -> ConversationListItem {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "revision": revision,
            "title": id,
            "preview": "",
            "provider_id": "provider",
            "model": "model",
            "message_count": 0,
            "created_at": 1,
            "updated_at": 1
        }))
        .unwrap()
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
    fn atomic_write_failure_keeps_the_previous_file_readable() {
        let dir = temp_dir();
        let path = dir.join("index.json");
        atomic_write(&path, "known-good", "test").unwrap();

        fail_atomic_replace_for_current_test(true);
        let result = atomic_write(&path, "replacement", "test");
        fail_atomic_replace_for_current_test(false);

        assert!(result.is_err(), "the injected replacement must fail");
        assert_eq!(fs::read_to_string(&path).unwrap(), "known-good");
        assert!(path.exists(), "the target must never be deleted first");
        assert!(
            fs::read_dir(&dir)
                .unwrap()
                .filter_map(Result::ok)
                .all(|entry| !entry.file_name().to_string_lossy().contains(".tmp.")),
            "a failed replacement must clean up its temporary file"
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn concurrent_temporary_paths_are_unique() {
        let target = Arc::new(PathBuf::from("C:/tmp/conv_same.json"));
        let paths = Arc::new(Mutex::new(Vec::new()));
        let threads: Vec<_> = (0..64)
            .map(|_| {
                let target = Arc::clone(&target);
                let paths = Arc::clone(&paths);
                thread::spawn(move || {
                    paths
                        .lock()
                        .unwrap()
                        .push(temporary_write_path(target.as_ref()));
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        let paths = paths.lock().unwrap();
        let unique: std::collections::HashSet<_> = paths.iter().collect();
        assert_eq!(paths.len(), 64);
        assert_eq!(unique.len(), paths.len());
        assert!(paths.iter().all(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(".conv_same.json.tmp."))
        }));
    }

    #[test]
    fn atomic_write_syncs_full_content_and_leaves_no_temp_file() {
        let dir = temp_dir();
        let path = dir.join("conv_a.json");
        // 大内容:一次 write_all + sync_all 之后 rename,读回来必须一字不少
        // (少了就说明数据还在页缓存里、rename 却已经生效——断电后就是 0 字节/截断文件)。
        let big = "x".repeat(1024 * 1024);
        atomic_write(&path, &big, "test").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap().len(), big.len());

        // 覆盖已有文件:整体替换,不留旧内容残尾。
        atomic_write(&path, "short", "test").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "short");

        let leftovers: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains(".tmp."))
            .collect();
        assert!(leftovers.is_empty(), "残留临时文件：{leftovers:?}");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn legacy_conversation_revision_defaults_to_zero() {
        let conversation: Conversation = serde_json::from_value(serde_json::json!({
            "id": "conv_legacy",
            "title": "legacy",
            "provider_id": "provider",
            "model": "model",
            "created_at": 1,
            "updated_at": 1,
            "messages": []
        }))
        .unwrap();
        assert_eq!(conversation.revision, 0);
        assert_eq!(ConversationListItem::from(&conversation).revision, Some(0));
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
        let index = ConversationIndex {
            conversations: vec![list_item("conv_a", Some(1)), list_item("conv_b", Some(1))],
        };
        // 索引覆盖全部文件(还多一个幽灵条目 conv_b)→ 信任
        assert!(index_covers_files(&index, &["conv_a".to_string()]));
        // 有文件(conv_c)不在索引 → 需补缺
        assert!(!index_covers_files(
            &index,
            &["conv_a".to_string(), "conv_c".to_string()]
        ));
    }

    /// 对账必须廉价:只比文件名,绝不反序列化对话正文。
    ///
    /// 这条挂了就意味着"侧栏刷新退化成全量扫盘"那个性能回退回来了——500 个会话的用户每点
    /// 一次侧栏就要同步解析几百 MB JSON。这里用"文件名合法但正文是坏 JSON"的会话当探针:
    /// 已在索引里的条目只要有人去读正文,就会被判损坏并从列表里消失。
    #[test]
    fn cheap_reconciliation_trusts_index_without_reading_conversation_bodies() {
        let dir = temp_dir();
        fs::write(
            dir.join("index.json"),
            serde_json::to_string(&ConversationIndex {
                conversations: vec![list_item("conv_a", Some(1))],
            })
            .unwrap(),
        )
        .unwrap();
        fs::write(dir.join("conv_a.json"), "{ not json at all").unwrap();

        let (index, healed) = load_index_or_scan_in_dir(&dir).unwrap();
        assert!(!healed);
        assert_eq!(index.conversations.len(), 1);
        assert_eq!(index.conversations[0].id, "conv_a");

        // 索引没覆盖的新文件只读那一份；已在索引里的会话仍然不读正文。
        fs::write(dir.join("conv_b.json"), "{ also broken").unwrap();
        let (index, healed) = load_index_or_scan_in_dir(&dir).unwrap();
        assert!(healed);
        assert_eq!(index.conversations.len(), 1);
        assert_eq!(index.conversations[0].id, "conv_a");

        // 且补缺只读不写:自愈落盘归持有 index_lock 的写路径,这里写回就会 lost update。
        assert_eq!(load_index_in_dir(&dir).unwrap().conversations.len(), 1);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_index_entries_are_merged_without_rereading_indexed_bodies() {
        let dir = temp_dir();
        fs::write(
            dir.join("index.json"),
            serde_json::to_string(&ConversationIndex {
                conversations: vec![list_item("conv_a", Some(1))],
            })
            .unwrap(),
        )
        .unwrap();
        // 已索引会话正文损坏：如果补缺时整目录重扫，conv_a 会被当成坏文件丢掉。
        fs::write(dir.join("conv_a.json"), "{ not json at all").unwrap();
        fs::write(
            dir.join("conv_b.json"),
            serde_json::json!({
                "id": "conv_b",
                "title": "new",
                "provider_id": "provider",
                "model": "model",
                "created_at": 2,
                "updated_at": 2,
                "messages": [{
                    "id": "msg_1",
                    "role": "user",
                    "content": "hello from the new conversation",
                    "timestamp": 2
                }]
            })
            .to_string(),
        )
        .unwrap();

        let (index, healed) = load_index_or_scan_in_dir(&dir).unwrap();
        assert!(healed);
        let mut ids: Vec<_> = index
            .conversations
            .iter()
            .map(|item| item.id.as_str())
            .collect();
        ids.sort();
        assert_eq!(ids, vec!["conv_a", "conv_b"]);
        let added = index
            .conversations
            .iter()
            .find(|item| item.id == "conv_b")
            .expect("merged missing conversation");
        assert_eq!(added.title, "new");
        assert_eq!(added.preview, "hello from the new conversation");

        assert_eq!(load_index_in_dir(&dir).unwrap().conversations.len(), 1);
        fs::remove_dir_all(&dir).ok();
    }

    fn search_item(id: &str, updated_at: i64, archived: bool) -> ConversationListItem {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "revision": 1,
            "title": id,
            "preview": "",
            "provider_id": "provider",
            "model": "model",
            "message_count": 0,
            "created_at": 1,
            "updated_at": updated_at,
            "archived": archived
        }))
        .unwrap()
    }

    #[test]
    fn search_ranks_unarchived_newest_first() {
        let ranked = rank_conversations_for_search(vec![
            search_item("conv_old", 1, false),
            search_item("conv_archived", 9, true),
            search_item("conv_new", 5, false),
        ]);
        let ids: Vec<_> = ranked.iter().map(|item| item.id.as_str()).collect();
        assert_eq!(ids, vec!["conv_new", "conv_old"]);
    }

    #[test]
    fn search_raw_prefilter_skips_serde_when_needle_absent() {
        let raw = r#"{"id":"conv_a","messages":[{"content":"你好世界"}]}"#;
        assert!(conversation_raw_might_contain(raw, "你好"));
        assert!(!conversation_raw_might_contain(raw, "不存在的词"));
        assert!(!conversation_raw_might_contain(raw, "hello"));
        assert!(conversation_raw_might_contain(
            r#"{"content":"HELLO WORLD"}"#,
            "hello"
        ));
        assert!(!conversation_raw_might_contain(
            r#"{"content":"HELLO WORLD"}"#,
            "missing"
        ));
        assert!(!conversation_raw_might_contain(raw, ""));
        assert!(!conversation_raw_might_contain(
            r#"{"content":"first line\nsecond line"}"#,
            "missing"
        ));
    }

    fn assert_search_prefilter_preserves_match(encoded_text: &str, needle: &str) {
        for field in ["content", "reasoning"] {
            let mut value = serde_json::json!({
                "id": "conv_search",
                "title": "Search regression",
                "provider_id": "provider",
                "model": "model",
                "created_at": 1,
                "updated_at": 1,
                "messages": [{
                    "id": "msg_search",
                    "role": "assistant",
                    "content": "",
                    "timestamp": 1
                }]
            });
            value["messages"][0][field] = serde_json::json!("__search_text__");
            // 保留输入的 JSON 转义写法，覆盖导入文件中非规范但合法的编码。
            let raw = value
                .to_string()
                .replace(r#""__search_text__""#, encoded_text);
            let conversation: Conversation = serde_json::from_str(&raw).unwrap();
            let needle_lower = needle.to_lowercase();
            assert!(
                messages_match(&conversation, &needle_lower),
                "fixture must match {field}: {encoded_text} / {needle:?}"
            );
            assert!(
                conversation_raw_might_contain(&raw, &needle_lower),
                "prefilter rejected {field}: {encoded_text} / {needle:?}"
            );
        }
    }

    #[test]
    fn search_raw_prefilter_preserves_json_escaped_matches() {
        for (encoded, needle) in [
            (r#""C:\\Users\\alice""#, r"c:\users\alice"),
            (r#""say \"hello\"""#, "\"hello\""),
            (r#""first\nsecond""#, "first\nsecond"),
            (r#""first\tsecond""#, "first\tsecond"),
            (r#""first\rsecond""#, "first\rsecond"),
            (r#""first\bsecond""#, "first\u{0008}second"),
            (r#""first\fsecond""#, "first\u{000c}second"),
            (r#""foo\/bar""#, "foo/bar"),
            (r#""\u4f60\u597d""#, "你好"),
            (r#""fo\u006fbar""#, "foobar"),
            (r#""\uD83D\uDE00""#, "😀"),
        ] {
            assert_search_prefilter_preserves_match(encoded, needle);
        }
    }

    #[test]
    fn search_raw_prefilter_preserves_unicode_case_matches() {
        for (encoded, needle) in [
            (r#""ПРИВЕТ""#, "привет"),
            (r#""É""#, "é"),
            (r#""ΟΣ""#, "ος"),
            (r#""\nΣ""#, "σ"),
            (r#""İ""#, "i\u{0307}"),
            (r#""\u00c9""#, "é"),
        ] {
            assert_search_prefilter_preserves_match(encoded, needle);
        }
    }
}

#[cfg(test)]
mod delete_side_artifact_tests {
    use super::*;

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kivio_del_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 不变式：副产物清理只回警告，绝不回错。
    ///
    /// 这条挂了就意味着「一个删不掉的工作区能中止整个删除」那个 bug 回来了：对话文件
    /// 和索引条目留在磁盘上，而 `load_index_or_scan` 认文件是真相源，下次刷新侧栏对话
    /// 就原样冒回来——用户看到的是「删了又回来，点好几次才掉」。
    #[test]
    fn side_artifact_cleanup_reports_warnings_instead_of_failing() {
        let root = temp_dir();

        // 正常目录：清掉，不报警。
        let workspace = root.join("workspace");
        fs::create_dir_all(workspace.join("node_modules")).unwrap();
        fs::write(workspace.join("node_modules/x.js"), "x").unwrap();
        let attachments = root.join("conv_x_attachments");
        fs::create_dir_all(&attachments).unwrap();

        let warnings = remove_conversation_side_artifacts(Some(&workspace), Some(&attachments));
        assert!(warnings.is_empty(), "干净情况不该有警告：{warnings:?}");
        assert!(!workspace.exists());
        assert!(!attachments.exists());

        // 路径存在但不是目录（删不掉的一种确定性替身）：出警告，不 panic、不中止。
        let bogus = root.join("workspace-is-a-file");
        fs::write(&bogus, "not a dir").unwrap();
        let warnings = remove_conversation_side_artifacts(Some(&bogus), None);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains("工作区"), "{warnings:?}");

        // 不存在的路径：静默跳过。
        assert!(remove_conversation_side_artifacts(
            Some(&root.join("gone")),
            Some(&root.join("nope"))
        )
        .is_empty());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn conversation_without_additional_directories_deserializes() {
        let conversation: crate::chat::types::Conversation =
            serde_json::from_value(serde_json::json!({
                "id": "conv_old",
                "revision": 1,
                "title": "legacy",
                "provider_id": "p",
                "model": "m",
                "created_at": 1,
                "updated_at": 1,
                "messages": []
            }))
            .unwrap();
        assert!(conversation.additional_directories.is_empty());
    }

    #[test]
    fn normalize_additional_directories_skips_primary_and_duplicates() {
        let root = temp_dir();
        let primary = root.join("primary");
        let extra = root.join("biz");
        fs::create_dir_all(&primary).unwrap();
        fs::create_dir_all(&extra).unwrap();
        let primary_s =
            crate::utils::strip_windows_verbatim_prefix(fs::canonicalize(&primary).unwrap())
                .to_string_lossy()
                .to_string();
        let extra_s =
            crate::utils::strip_windows_verbatim_prefix(fs::canonicalize(&extra).unwrap())
                .to_string_lossy()
                .to_string();

        let out = normalize_additional_directories(
            vec![
                AdditionalDirectory {
                    path: extra_s.clone(),
                    name: Some("biz".to_string()),
                },
                AdditionalDirectory {
                    path: primary_s.clone(),
                    name: None,
                },
                AdditionalDirectory {
                    path: extra_s.clone(),
                    name: Some("dup".to_string()),
                },
            ],
            Some(&primary_s),
        )
        .unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].path, extra_s);
        assert_eq!(out[0].name.as_deref(), Some("biz"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn normalize_additional_directories_errors_on_missing_path() {
        let missing = std::env::temp_dir().join("kivio-additional-dir-does-not-exist-xyz");
        let err = normalize_additional_directories(
            vec![AdditionalDirectory {
                path: missing.to_string_lossy().to_string(),
                name: None,
            }],
            None,
        )
        .unwrap_err();
        assert!(err.contains("不存在"), "{err}");
    }

    #[test]
    fn normalize_additional_directories_caps_at_eight() {
        let root = temp_dir();
        let entries: Vec<_> = (0..9)
            .map(|i| {
                let dir = root.join(format!("d{i}"));
                fs::create_dir_all(&dir).unwrap();
                AdditionalDirectory {
                    path: dir.to_string_lossy().to_string(),
                    name: None,
                }
            })
            .collect();
        let err = normalize_additional_directories(entries, None).unwrap_err();
        assert!(err.contains("8"), "{err}");
        let _ = fs::remove_dir_all(&root);
    }
}

#[cfg(test)]
mod reorder_tests {
    use super::*;

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn order(items: &[&'static str], want: &[&str]) -> Vec<&'static str> {
        reorder_by_ids(items.to_vec(), &ids(want), |s| *s)
    }

    #[test]
    fn applies_requested_order() {
        assert_eq!(order(&["a", "b", "c"], &["c", "a", "b"]), ["c", "a", "b"]);
    }

    #[test]
    fn ignores_unknown_ids_and_duplicates() {
        // 前端拿的是旧快照：ids 里提到了已删除的 "gone"，还重复了 "a"。
        assert_eq!(
            order(&["a", "b"], &["gone", "b", "a", "a"]),
            ["b", "a"],
            "认不出的 id 应被忽略，重复 id 只认第一次"
        );
    }

    #[test]
    fn unmentioned_items_keep_relative_order_and_go_first() {
        // 别处新建的 "new" 前端还不知道；新建是 insert(0)，所以它应留在最前。
        assert_eq!(order(&["new", "a", "b"], &["b", "a"]), ["new", "b", "a"]);
    }

    #[test]
    fn is_idempotent() {
        let once = order(&["a", "b", "c"], &["b", "c", "a"]);
        let twice = reorder_by_ids(once.clone(), &ids(&once), |s| *s);
        assert_eq!(once, twice);
    }
}
