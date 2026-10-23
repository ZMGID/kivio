use super::Shell;
use gpui_kit::{AppContext, TestAppContext, WindowOptions};

// Primary owner for cross-conversation isolation and runtime binding. Exercises
// real input delivery and view subscriptions, not a copied state machine.
#[gpui_kit::test]
fn switching_conversations_retains_identity_and_runtime_binding(cx: &mut TestAppContext) {
    let data = tempfile::tempdir().unwrap();
    let (window, shell) = cx.update(|cx| {
        gpui_kit::init(cx);
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| {
                let shell = Shell::new(data.path().to_owned(), window, cx);
                shell
                    .current()
                    .view
                    .update(cx, |view, cx| view.load_sample(0, cx));
                shell
            })
        })
        .unwrap()
    });
    cx.run_until_parked();
    cx.simulate_input(window, "第一条消息");
    cx.simulate_keystrokes(window, "enter");
    cx.run_until_parked();
    let first = shell.read_with(cx, |shell, _| {
        assert!(shell.current().locked);
        assert_eq!(shell.current().title, "第一条消息");
        shell.current().view.clone()
    });
    cx.simulate_keystrokes(window, "secondary-n");
    shell.read_with(cx, |shell, _| {
        assert_eq!(shell.conversations.len(), 2);
        assert!(!shell.current().locked);
        assert_ne!(shell.current().view, first);
    });
    cx.simulate_input(window, "第二条对话的草稿");
    cx.simulate_keystrokes(window, "secondary-[");
    shell.read_with(cx, |shell, cx| {
        assert_eq!(shell.current().view, first);
        assert!(shell.current().locked);
        assert!(shell.current().view.read(cx).is_streaming(cx));
    });
    cx.simulate_keystrokes(window, "secondary-]");
    shell.read_with(cx, |shell, cx| {
        assert_ne!(shell.current().view, first);
        assert_eq!(
            shell.current().view.read(cx).snapshot(cx).1,
            "第二条对话的草稿"
        );
    });
    cx.simulate_keystrokes(window, "secondary-[");
    cx.simulate_keystrokes(window, "secondary-n");
    let branched = shell.read_with(cx, |shell, _| shell.current().view.clone());
    cx.simulate_keystrokes(window, "secondary-]");
    shell.read_with(cx, |shell, _| assert_eq!(shell.current().view, branched));
}

// ADR 0004: editing a group must not turn manual ordering into recency ordering.
#[gpui_kit::test]
fn group_editor_preserves_manual_order_and_new_chat_membership(cx: &mut TestAppContext) {
    let data = tempfile::tempdir().unwrap();
    let (window, shell) = cx.update(|cx| {
        gpui_kit::init(cx);
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| {
                let shell = Shell::new(data.path().to_owned(), window, cx);
                shell
                    .current()
                    .view
                    .update(cx, |view, cx| view.load_sample(0, cx));
                shell
            })
        })
        .unwrap()
    });
    cx.run_until_parked();
    for title in ["项目 A", "项目 B"] {
        window
            .update(cx, |_, window, cx| {
                shell.update(cx, |shell, cx| shell.edit_group(None, true, window, cx))
            })
            .unwrap();
        cx.simulate_input(window, title);
        cx.simulate_keystrokes(window, "enter");
        cx.run_until_parked();
    }
    window
        .update(cx, |_, window, cx| {
            shell.update(cx, |shell, cx| shell.edit_group(Some(0), true, window, cx))
        })
        .unwrap();
    cx.simulate_keystrokes(window, "secondary-a");
    cx.simulate_input(window, "项目 A 改名");
    cx.simulate_keystrokes(window, "enter");
    cx.run_until_parked();
    shell.read_with(cx, |shell, _| {
        assert_eq!(
            shell
                .groups
                .iter()
                .map(|group| group.id)
                .collect::<Vec<_>>(),
            [1, 0]
        );
        assert_eq!(shell.groups[1].name, "项目 A 改名");
    });
    window
        .update(cx, |_, window, cx| {
            shell.update(cx, |shell, cx| shell.new_chat(window, cx))
        })
        .unwrap();
    shell.read_with(cx, |shell, _| assert_eq!(shell.current().project, Some(0)));
    window
        .update(cx, |_, window, cx| {
            shell.update(cx, |shell, cx| {
                shell.drop_group(
                    &super::library::DragGroup {
                        id: 0,
                        project: true,
                        name: "项目 A 改名".into(),
                    },
                    Some(1),
                    window,
                    cx,
                );
            })
        })
        .unwrap();
    cx.run_until_parked();
    shell.read_with(cx, |shell, _| {
        assert_eq!(
            shell
                .groups
                .iter()
                .map(|group| group.id)
                .collect::<Vec<_>>(),
            [0, 1]
        );
        assert_eq!(shell.current().project, Some(0));
    });
    let (_reopened_window, reopened) = cx.update(|cx| {
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| {
                let shell = Shell::new(data.path().to_owned(), window, cx);
                shell
                    .current()
                    .view
                    .update(cx, |view, cx| view.load_sample(0, cx));
                shell
            })
        })
        .unwrap()
    });
    cx.run_until_parked();
    reopened.read_with(cx, |shell, _| {
        assert_eq!(
            shell
                .groups
                .iter()
                .map(|g| g.name.as_ref())
                .collect::<Vec<_>>(),
            ["项目 A 改名", "项目 B"]
        );
    });
    // A failed filesystem commit must leave the dialog/draft and visible rows intact.
    std::fs::remove_file(data.path().join("projects.json")).unwrap();
    std::fs::create_dir(data.path().join("projects.json")).unwrap();
    window
        .update(cx, |_, window, cx| {
            shell.update(cx, |shell, cx| {
                shell.edit_group(Some(0), true, window, cx);
            })
        })
        .unwrap();
    cx.simulate_keystrokes(window, "secondary-a");
    cx.simulate_input(window, "保存失败的草稿");
    cx.simulate_keystrokes(window, "enter");
    cx.run_until_parked();
    shell.read_with(cx, |shell, cx| {
        let editor = shell
            .group_editor
            .as_ref()
            .expect("failed save keeps editor open");
        assert!(editor.error.is_some());
        assert_eq!(editor.name.read(cx).value(), "保存失败的草稿");
        assert_eq!(shell.groups[0].name, "项目 A 改名");
    });
}

// Search must route native arrow/Enter events through the modal, leaving the
// conversation and its draft alone until a valid result is confirmed.
#[gpui_kit::test]
fn search_keyboard_selects_results_and_keeps_empty_results_open(cx: &mut TestAppContext) {
    let data = tempfile::tempdir().unwrap();
    let (window, shell) = cx.update(|cx| {
        gpui_kit::init(cx);
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| {
                let shell = Shell::new(data.path().to_owned(), window, cx);
                shell
                    .current()
                    .view
                    .update(cx, |view, cx| view.load_sample(0, cx));
                shell
            })
        })
        .unwrap()
    });
    cx.run_until_parked();
    cx.simulate_input(window, "第一条消息");
    cx.simulate_keystrokes(window, "enter secondary-n");
    shell.update(cx, |shell, cx| {
        shell
            .current()
            .view
            .update(cx, |view, cx| view.set_sample_mode(true, cx))
    });
    cx.simulate_input(window, "保留这份草稿");
    cx.simulate_keystrokes(window, "secondary-f down enter");
    shell.read_with(cx, |shell, _| {
        assert_eq!(shell.active, 0);
        assert!(!shell.searching);
    });
    cx.simulate_keystrokes(window, "secondary-f");
    cx.simulate_input(window, "不存在的对话");
    cx.simulate_keystrokes(window, "enter");
    cx.run_until_parked();
    shell.read_with(cx, |shell, _| assert!(shell.searching));
    cx.simulate_keystrokes(window, "escape");
    shell.read_with(cx, |shell, cx| {
        assert!(!shell.searching);
        assert_eq!(shell.active, 0);
        assert!(shell.search.read(cx).value().is_empty());
    });
    window
        .update(cx, |_, window, cx| {
            shell.update(cx, |shell, cx| shell.select(1, window, cx));
        })
        .unwrap();
    cx.simulate_keystrokes(window, "enter");
    cx.run_until_parked();
    shell.read_with(cx, |shell, _| {
        assert_eq!(shell.current().title, "保留这份草稿")
    });
}

// The input subscription, stream completion and metadata actions must all feed
// the same durable owner; reopening must restore both history and the next draft.
#[gpui_kit::test]
fn conversation_input_and_metadata_survive_reopening(cx: &mut TestAppContext) {
    let data = tempfile::tempdir().unwrap();
    let (window, shell) = cx.update(|cx| {
        gpui_kit::init(cx);
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| {
                let shell = Shell::new(data.path().to_owned(), window, cx);
                shell
                    .current()
                    .view
                    .update(cx, |view, cx| view.load_sample(0, cx));
                shell
            })
        })
        .unwrap()
    });
    cx.run_until_parked();
    cx.simulate_input(window, "保存这条提问 🌿");
    cx.run_until_parked();
    let id = shell.read_with(cx, |shell, _| shell.current().record.id.clone());
    let drafts: serde_json::Value =
        serde_json::from_slice(&std::fs::read(data.path().join("drafts.json")).unwrap()).unwrap();
    assert_eq!(drafts[&id], "保存这条提问 🌿");

    cx.simulate_keystrokes(window, "enter");
    cx.run_until_parked();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(80));
    cx.run_until_parked();
    cx.simulate_keystrokes(window, "escape");
    cx.simulate_input(window, "下一条未发送的草稿");
    cx.run_until_parked();
    window
        .update(cx, |_, window, cx| {
            window.dispatch_action(Box::new(super::RenameChat), cx);
        })
        .unwrap();
    cx.run_until_parked();
    cx.simulate_keystrokes(window, "secondary-a");
    cx.simulate_input(window, "恢复后的对话");
    cx.simulate_keystrokes(window, "enter");
    cx.run_until_parked();
    window
        .update(cx, |_, window, cx| {
            window.dispatch_action(Box::new(super::ArchiveChat), cx);
        })
        .unwrap();
    cx.run_until_parked();

    let stored = crate::conversation_files::ConversationStore::new(data.path().to_owned())
        .load(&id)
        .unwrap();
    assert_eq!(stored.title, "恢复后的对话");
    assert!(stored.archived);
    assert_eq!(stored.messages.len(), 2);
    assert_eq!(stored.messages[0].content, "保存这条提问 🌿");
    assert!(!stored.messages[1].content.is_empty());
    assert_eq!(
        stored.messages[1].stream_outcome.as_deref(),
        Some("cancelled")
    );
    let (_, reopened) = cx.update(|cx| {
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| {
                let shell = Shell::new(data.path().to_owned(), window, cx);
                shell
                    .current()
                    .view
                    .update(cx, |view, cx| view.load_sample(0, cx));
                shell
            })
        })
        .unwrap()
    });
    cx.run_until_parked();
    reopened.read_with(cx, |shell, cx| {
        let chat = shell
            .conversations
            .iter()
            .find(|chat| chat.record.id == id)
            .unwrap();
        assert_eq!(chat.title, "恢复后的对话");
        assert!(chat.archived);
        let (messages, draft) = chat.view.read(cx).snapshot(cx);
        assert_eq!(draft, "下一条未发送的草稿");
        assert_eq!(messages[0].id, stored.messages[0].id);
        assert_eq!(messages[1].content, stored.messages[1].content);
        assert_eq!(messages[1].stream_outcome.as_deref(), Some("cancelled"));
        assert!(shell.persistence.error.is_none());
    });
}

// Drafts are a second durable write after the history commit. Its failure must
// retain the confirmed CAS revision, so retry saves the live draft, not a stale
// snapshot. The production view must also retain opaque imported history fields.
#[gpui_kit::test]
fn draft_retry_preserves_committed_revision_and_rich_history(cx: &mut TestAppContext) {
    use gpui_kit::test::TestWindowExt;

    let data = tempfile::tempdir().unwrap();
    // Exercise the actual retry control with stable notification geometry.
    std::fs::write(
        data.path().join("appearance.json"),
        r#"{"reduceMotion":true}"#,
    )
    .unwrap();
    let path = data.path().join("conv_legacy.json");
    let original = serde_json::json!({
        "id":"conv_legacy", "revision":7, "title":"原有对话", "provider_id":"original-provider",
        "model":"original-model", "created_at":1, "updated_at":2,
        "agent_runtime":{"kind":"external", "externalAgentId":"claude", "externalModel":"sonnet"},
        "messages":[{"id":"msg_original", "role":"assistant", "content":"已保存的回答", "timestamp":2,
            "model_messages":[{"role":"assistant", "content":[{"type":"anthropic_thinking", "thinking":"plan", "signature":"signed-history"}]}],
            "usage":{"input_tokens":11,"output_tokens":5,"total_tokens":16},
            "group_id":"group_original"
        }],
        "group_selections":{"group_original":"msg_original"}
    });
    let original_bytes = serde_json::to_vec(&original).unwrap();
    std::fs::write(&path, &original_bytes).unwrap();
    let (window, shell) = cx.update(|cx| {
        gpui_kit::init(cx);
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| {
                let shell = Shell::new(data.path().to_owned(), window, cx);
                shell
                    .current()
                    .view
                    .update(cx, |view, cx| view.load_sample(0, cx));
                shell
            })
        })
        .unwrap()
    });
    cx.run_until_parked();
    assert_eq!(
        std::fs::read(&path).unwrap(),
        original_bytes,
        "opening history must not rewrite it"
    );
    let drafts_path = data.path().join("drafts.json");
    cx.simulate_input(window, "保存失败时仍然保留的草稿");
    cx.run_until_parked();
    std::fs::remove_file(&drafts_path).unwrap();
    std::fs::create_dir(&drafts_path).unwrap();
    window
        .update(cx, |_, window, cx| {
            window.dispatch_action(Box::new(super::RenameChat), cx);
        })
        .unwrap();
    cx.run_until_parked();
    cx.simulate_keystrokes(window, "secondary-a");
    cx.simulate_input(window, "保留完整历史的重命名");
    cx.simulate_keystrokes(window, "enter");
    cx.run_until_parked();
    let committed: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert!(committed["revision"].as_u64().unwrap() > 7);
    shell.read_with(cx, |shell, cx| {
        assert!(shell.persistence.error.is_some());
        assert_eq!(
            shell.current().record.revision,
            committed["revision"].as_u64().unwrap()
        );
        assert_eq!(
            shell.current().view.read(cx).snapshot(cx).1,
            "保存失败时仍然保留的草稿"
        );
    });
    std::fs::remove_dir(&drafts_path).unwrap();
    window
        .update(cx, |_, window, cx| {
            window.render_frame(cx);
            window.click("retry-save", cx)
        })
        .unwrap();
    cx.run_until_parked();
    shell.read_with(cx, |shell, _| {
        assert!(
            shell.persistence.error.is_none(),
            "retry must not conflict with the already committed revision: {:?}",
            shell.persistence.error
        );
        assert!(!shell.current().dirty);
    });
    let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(saved["title"], "保留完整历史的重命名");
    assert_eq!(saved["agent_runtime"]["kind"], "external");
    assert_eq!(saved["agent_runtime"]["externalAgentId"], "claude");
    assert_eq!(saved["agent_runtime"]["externalModel"], "sonnet");
    assert_eq!(
        saved["messages"][0]["model_messages"],
        original["messages"][0]["model_messages"]
    );
    assert_eq!(saved["messages"][0]["usage"]["total_tokens"], 16);
    assert_eq!(saved["group_selections"], original["group_selections"]);
    let drafts: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&drafts_path).unwrap()).unwrap();
    assert_eq!(drafts["conv_legacy"], "保存失败时仍然保留的草稿");
}

// Quitting from the keyboard must use the same save barrier as window close.
#[gpui_kit::test]
fn quit_action_enters_the_conversation_save_barrier(cx: &mut TestAppContext) {
    let data = tempfile::tempdir().unwrap();
    let (window, shell) = cx.update(|cx| {
        gpui_kit::init(cx);
        cx.bind_keys([gpui_kit::KeyBinding::new("secondary-q", crate::Quit, None)]);
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| Shell::new(data.path().to_owned(), window, cx))
        })
        .unwrap()
    });
    cx.run_until_parked();
    shell.update(cx, |shell, cx| {
        shell
            .current()
            .view
            .update(cx, |view, cx| view.load_sample(0, cx))
    });
    cx.simulate_input(window, "正在生成");
    cx.simulate_keystrokes(window, "enter");
    cx.simulate_input(window, "退出前的草稿");
    cx.simulate_keystrokes(window, "secondary-q");
    shell.read_with(cx, |shell, cx| {
        assert!(!shell.current().view.read(cx).is_streaming(cx))
    });
    cx.run_until_parked();
    let drafts: serde_json::Value =
        serde_json::from_slice(&std::fs::read(data.path().join("drafts.json")).unwrap()).unwrap();
    assert!(
        drafts
            .as_object()
            .unwrap()
            .values()
            .any(|value| value.as_str() == Some("退出前的草稿"))
    );
}
