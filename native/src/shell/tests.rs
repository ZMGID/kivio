use super::Shell;
use gpui_kit::{AppContext, TestAppContext, WindowOptions};

// Primary owner for cross-conversation isolation and runtime binding. Exercises
// real input delivery and view subscriptions, not a copied state machine.
#[gpui_kit::test]
fn switching_conversations_retains_identity_and_runtime_binding(cx: &mut TestAppContext) {
    let (window, shell) = cx.update(|cx| {
        gpui_kit::init(cx);
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| Shell::new(window, cx))
        })
        .unwrap()
    });
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
    window
        .update(cx, |_, window, cx| {
            shell.update(cx, |shell, cx| shell.select(0, window, cx))
        })
        .unwrap();
    shell.read_with(cx, |shell, cx| {
        assert_eq!(shell.current().view, first);
        assert!(shell.current().locked);
        assert!(shell.current().view.read(cx).is_streaming(cx));
    });
}

// ADR 0004: editing a group must not turn manual ordering into recency ordering.
#[gpui_kit::test]
fn group_editor_preserves_manual_order_and_new_chat_membership(cx: &mut TestAppContext) {
    let (window, shell) = cx.update(|cx| {
        gpui_kit::init(cx);
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| Shell::new(window, cx))
        })
        .unwrap()
    });
    for title in ["项目 A", "项目 B"] {
        window
            .update(cx, |_, window, cx| {
                shell.update(cx, |shell, cx| shell.edit_group(None, true, window, cx))
            })
            .unwrap();
        cx.simulate_input(window, title);
        cx.simulate_keystrokes(window, "enter");
    }
    window
        .update(cx, |_, window, cx| {
            shell.update(cx, |shell, cx| shell.edit_group(Some(0), true, window, cx))
        })
        .unwrap();
    cx.simulate_keystrokes(window, "secondary-a");
    cx.simulate_input(window, "项目 A 改名");
    cx.simulate_keystrokes(window, "enter");
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
    shell.update(cx, |shell, cx| {
        shell.drop_group(
            &super::library::DragGroup {
                id: 0,
                project: true,
                name: "项目 A 改名".into(),
            },
            Some(1),
            cx,
        );
    });
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
}

// Search must route native arrow/Enter events through the modal, leaving the
// conversation and its draft alone until a valid result is confirmed.
#[gpui_kit::test]
fn search_keyboard_selects_results_and_keeps_empty_results_open(cx: &mut TestAppContext) {
    let (window, shell) = cx.update(|cx| {
        gpui_kit::init(cx);
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| Shell::new(window, cx))
        })
        .unwrap()
    });
    cx.simulate_input(window, "第一条消息");
    cx.simulate_keystrokes(window, "enter secondary-n");
    cx.simulate_input(window, "保留这份草稿");
    cx.simulate_keystrokes(window, "secondary-f down enter");
    shell.read_with(cx, |shell, _| {
        assert_eq!(shell.active, 0);
        assert!(!shell.searching);
    });
    cx.simulate_keystrokes(window, "secondary-f");
    cx.simulate_input(window, "不存在的对话");
    cx.simulate_keystrokes(window, "enter");
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
    shell.read_with(cx, |shell, _| {
        assert_eq!(shell.current().title, "保留这份草稿")
    });
}
