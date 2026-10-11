use std::collections::HashSet;
use std::sync::Arc;

use super::observe::classify_native_reply;
use super::runtime::BotCollaboration;
use super::store::delivery_record_path;
use super::types::{
    bot_conversation_id, Actor, DeliveryKind, DeliveryStatus, Enqueue, MemberInput, PendingKind,
    PendingStatus, ReceiptState, RoomStatus, RunFinish, GROUP_STOP_REASON, INTERRUPT_REASON,
    MAX_PARALLEL_DISPATCH, NO_FABRICATED_REPLY,
};

fn service(dir: &std::path::Path) -> BotCollaboration {
    BotCollaboration::load(dir.to_path_buf())
}

fn send(
    svc: &BotCollaboration,
    from_id: &str,
    from_name: &str,
    to_id: &str,
    to_name: &str,
    body: &str,
    room_id: Option<String>,
    kind: DeliveryKind,
    approved: bool,
    now: i64,
) -> super::types::Delivery {
    svc.enqueue(
        Enqueue {
            from: Actor::bot(from_id, from_name),
            to_bot_id: to_id.into(),
            to_name: to_name.into(),
            room_id,
            kind,
            body: body.into(),
            exchange_id: None,
            parent_delivery_id: None,
            sensitive: false,
            approved,
            attachment_ids: Vec::new(),
            attachment_sources: Vec::new(),
        },
        now,
    )
    .unwrap()
}

#[test]
fn busy_recipient_replies_once_with_native_output() {
    let dir = tempfile::tempdir().unwrap();
    let svc = service(dir.path());
    let delivery = send(
        &svc,
        "a",
        "Ava",
        "b",
        "Bo",
        "请看一下草案",
        None,
        DeliveryKind::Peer,
        true,
        1_000,
    );
    let busy = HashSet::from([bot_conversation_id("b")]);
    assert!(svc.claim_next(&busy, "t1".into(), 1_001).unwrap().is_none());
    assert_eq!(
        svc.get_delivery(&delivery.id).unwrap().status,
        DeliveryStatus::Queued
    );

    let claimed = svc
        .claim_next(&HashSet::new(), "t2".into(), 1_002)
        .unwrap()
        .unwrap();
    assert_eq!(claimed.status, DeliveryStatus::Claimed);
    assert!(svc
        .claim_next(&HashSet::new(), "t3".into(), 1_003)
        .unwrap()
        .is_none());
    assert!(svc.begin_send(&claimed.id, "t2", 1_004).unwrap());
    assert!(!svc.begin_send(&claimed.id, "t2", 1_005).unwrap());

    let report = svc
        .finish(
            &claimed.id,
            "t2",
            RunFinish::Succeeded {
                native_output: "草案可以，补一节风险".into(),
                model_enqueued: false,
            },
            1_006,
        )
        .unwrap();
    let reply = report.follow_up.expect("one reply");
    assert_eq!(reply.body, "草案可以，补一节风险");
    assert_ne!(reply.body, "请看一下草案");
    assert_eq!(reply.kind, DeliveryKind::Reply);
    assert_eq!(reply.depth, 1);
    match &reply.from {
        Actor::Bot { id, name } => {
            assert_eq!(id, "b");
            assert_eq!(name, "Bo");
        }
        Actor::User => panic!("reply must keep the recipient identity"),
    }

    let busy_sender = HashSet::from([bot_conversation_id("a")]);
    assert!(svc
        .claim_next(&busy_sender, "t4".into(), 1_007)
        .unwrap()
        .is_none());
    let claimed_reply = svc
        .claim_next(&HashSet::new(), "t5".into(), 1_008)
        .unwrap()
        .unwrap();
    assert_eq!(claimed_reply.to_bot_id, "a");
    assert!(svc.begin_send(&claimed_reply.id, "t5", 1_009).unwrap());
    let second = svc
        .finish(
            &claimed_reply.id,
            "t5",
            RunFinish::Succeeded {
                native_output: "收到".into(),
                model_enqueued: false,
            },
            1_010,
        )
        .unwrap();
    assert!(second.follow_up.is_none(), "a reply must not chain");

    let original = svc.get_delivery(&delivery.id).unwrap();
    assert_eq!(
        original.native_output.as_deref(),
        Some("草案可以，补一节风险")
    );
    assert_eq!(original.status, DeliveryStatus::Succeeded);
    assert!(original
        .receipts
        .iter()
        .any(|receipt| receipt.state == ReceiptState::Completed));
    let timeline = svc.exchange(&delivery.exchange_id).unwrap();
    assert!(timeline.entries.iter().any(|entry| {
        entry.body == "请看一下草案" && matches!(&entry.actor, Actor::Bot { id, .. } if id == "a")
    }));
    assert!(timeline
        .entries
        .iter()
        .any(|entry| entry.body == "草案可以，补一节风险"));
    assert!(timeline.entries.iter().any(|entry| entry.body == "收到"));
}

#[test]
fn model_enqueued_reply_is_not_duplicated() {
    let dir = tempfile::tempdir().unwrap();
    let svc = service(dir.path());
    let delivery = send(
        &svc,
        "a",
        "Ava",
        "b",
        "Bo",
        "问一句",
        None,
        DeliveryKind::Peer,
        true,
        1,
    );
    let claimed = svc
        .claim_next(&HashSet::new(), "tok".into(), 2)
        .unwrap()
        .unwrap();
    svc.begin_send(&claimed.id, "tok", 3).unwrap();
    let report = svc
        .finish(
            &claimed.id,
            "tok",
            RunFinish::Succeeded {
                native_output: "模型已经调用了回复工具".into(),
                model_enqueued: true,
            },
            4,
        )
        .unwrap();
    assert!(report.follow_up.is_none());
    assert_eq!(
        svc.get_delivery(&delivery.id)
            .unwrap()
            .native_output
            .as_deref(),
        Some("模型已经调用了回复工具")
    );
    assert_eq!(svc.snapshot().deliveries.len(), 1);
}

#[test]
fn assistant_room_targets_the_member_thread() {
    let dir = tempfile::tempdir().unwrap();
    let svc = service(dir.path());
    let room = svc
        .create_room(
            "工作",
            vec![
                MemberInput {
                    bot_id: "asst_ava".into(),
                    name: "Ava".into(),
                },
                MemberInput {
                    bot_id: "asst_bo".into(),
                    name: "Bo".into(),
                },
                MemberInput {
                    bot_id: "asst_cyd".into(),
                    name: "Cyd".into(),
                },
            ],
            1,
        )
        .unwrap();
    let ava = room
        .members
        .iter()
        .find(|member| member.bot_id == "asst_ava")
        .unwrap();
    assert_eq!(
        ava.conversation_id,
        format!("conv_bot_member_asst_ava@{}", room.id)
    );
    let delivery = svc
        .enqueue(
            Enqueue {
                from: Actor::User,
                to_bot_id: "asst_bo".into(),
                to_name: "Bo".into(),
                room_id: Some(room.id.clone()),
                kind: DeliveryKind::Mention,
                body: "看这一段".into(),
                exchange_id: None,
                parent_delivery_id: None,
                sensitive: false,
                approved: true,
                attachment_ids: Vec::new(),
                attachment_sources: Vec::new(),
            },
            2,
        )
        .unwrap();
    assert_eq!(
        delivery.to_conversation_id,
        format!("conv_bot_member_asst_bo@{}", room.id)
    );
    assert_ne!(delivery.to_conversation_id, bot_conversation_id("asst_bo"));
}

#[test]
fn one_bot_stays_busy_across_private_and_group_threads() {
    let dir = tempfile::tempdir().unwrap();
    let svc = service(dir.path());
    let room = svc
        .create_room(
            "工作",
            vec![
                MemberInput {
                    bot_id: "asst_ava".into(),
                    name: "Ava".into(),
                },
                MemberInput {
                    bot_id: "asst_bo".into(),
                    name: "Bo".into(),
                },
            ],
            1,
        )
        .unwrap();
    let private_delivery = svc
        .enqueue(
            Enqueue {
                from: Actor::User,
                to_bot_id: "asst_bo".into(),
                to_name: "Bo".into(),
                room_id: None,
                kind: DeliveryKind::Peer,
                body: "私聊里的一件".into(),
                exchange_id: None,
                parent_delivery_id: None,
                sensitive: false,
                approved: true,
                attachment_ids: Vec::new(),
                attachment_sources: Vec::new(),
            },
            2,
        )
        .unwrap();
    let group_delivery = svc
        .enqueue(
            Enqueue {
                from: Actor::User,
                to_bot_id: "asst_bo".into(),
                to_name: "Bo".into(),
                room_id: Some(room.id),
                kind: DeliveryKind::Mention,
                body: "群线程里的一件".into(),
                exchange_id: None,
                parent_delivery_id: None,
                sensitive: false,
                approved: true,
                attachment_ids: Vec::new(),
                attachment_sources: Vec::new(),
            },
            3,
        )
        .unwrap();
    assert_ne!(
        private_delivery.to_conversation_id,
        group_delivery.to_conversation_id
    );
    let claimed = svc
        .claim_next(&HashSet::new(), "private".into(), 4)
        .unwrap()
        .unwrap();
    assert_eq!(claimed.id, private_delivery.id);
    assert!(svc
        .claim_next(&HashSet::new(), "group".into(), 5)
        .unwrap()
        .is_none());
}

#[test]
fn later_delivery_does_not_rewrite_an_unrelated_record() {
    let dir = tempfile::tempdir().unwrap();
    let svc = service(dir.path());
    let first = send(
        &svc,
        "asst_ava",
        "Ava",
        "asst_bo",
        "Bo",
        "第一件",
        None,
        DeliveryKind::Peer,
        true,
        1,
    );
    let path = delivery_record_path(dir.path(), &first.id);
    let before = std::fs::read(&path).unwrap();
    send(
        &svc,
        "asst_ava",
        "Ava",
        "asst_cyd",
        "Cyd",
        "另一件",
        None,
        DeliveryKind::Peer,
        true,
        2,
    );
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert!(!dir.path().join("collaboration.json").exists());
}

#[test]
fn room_of_three_bots_keeps_each_identity() {
    let dir = tempfile::tempdir().unwrap();
    let svc = service(dir.path());
    let room = svc
        .create_room(
            "工作",
            vec![
                MemberInput {
                    bot_id: "asst_ava".into(),
                    name: "Ava".into(),
                },
                MemberInput {
                    bot_id: "asst_bo".into(),
                    name: "Bo".into(),
                },
                MemberInput {
                    bot_id: "asst_cyd".into(),
                    name: "Cyd".into(),
                },
            ],
            1,
        )
        .unwrap();
    assert_eq!(room.members.len(), 3);

    let first = svc
        .enqueue(
            Enqueue {
                from: Actor::User,
                to_bot_id: "asst_ava".into(),
                to_name: "Ava".into(),
                room_id: Some(room.id.clone()),
                kind: DeliveryKind::Mention,
                body: "起草".into(),
                exchange_id: None,
                parent_delivery_id: None,
                sensitive: false,
                approved: true,
                attachment_ids: Vec::new(),
                attachment_sources: Vec::new(),
            },
            2,
        )
        .unwrap();
    let claimed = svc
        .claim_next(&HashSet::new(), "ava-tok".into(), 3)
        .unwrap()
        .unwrap();
    svc.begin_send(&claimed.id, "ava-tok", 4).unwrap();
    let report = svc
        .finish(
            &first.id,
            "ava-tok",
            RunFinish::Succeeded {
                native_output: "草稿甲".into(),
                model_enqueued: false,
            },
            5,
        )
        .unwrap();
    assert!(report.follow_up.is_none());

    let second = svc
        .enqueue(
            Enqueue {
                from: Actor::User,
                to_bot_id: "asst_bo".into(),
                to_name: "Bo".into(),
                room_id: Some(room.id.clone()),
                kind: DeliveryKind::Mention,
                body: "复核".into(),
                exchange_id: None,
                parent_delivery_id: None,
                sensitive: false,
                approved: true,
                attachment_ids: Vec::new(),
                attachment_sources: Vec::new(),
            },
            6,
        )
        .unwrap();
    svc.claim_next(&HashSet::new(), "bo-tok".into(), 7).unwrap();
    svc.begin_send(&second.id, "bo-tok", 8).unwrap();
    svc.finish(
        &second.id,
        "bo-tok",
        RunFinish::Succeeded {
            native_output: "复核乙".into(),
            model_enqueued: false,
        },
        9,
    )
    .unwrap();

    let room = svc.room(&room.id).unwrap();
    let names: Vec<&str> = room
        .members
        .iter()
        .map(|member| member.name.as_str())
        .collect();
    assert_eq!(names, ["Ava", "Bo", "Cyd"]);
    assert!(room
        .log
        .iter()
        .any(|entry| { matches!(entry.actor, Actor::User) && entry.body == "起草" }));
    assert!(room.log.iter().any(|entry| {
        matches!(&entry.actor, Actor::Bot { name, .. } if name == "Ava") && entry.body == "草稿甲"
    }));
    assert!(room.log.iter().any(|entry| {
        matches!(&entry.actor, Actor::Bot { name, .. } if name == "Bo") && entry.body == "复核乙"
    }));
}

#[test]
fn group_stop_cancels_only_that_room_cycle() {
    let dir = tempfile::tempdir().unwrap();
    let svc = service(dir.path());
    let room_a = svc
        .create_room(
            "甲",
            vec![MemberInput {
                bot_id: "asst_ava".into(),
                name: "Ava".into(),
            }],
            1,
        )
        .unwrap();
    let room_b = svc
        .create_room(
            "乙",
            vec![MemberInput {
                bot_id: "asst_bo".into(),
                name: "Bo".into(),
            }],
            1,
        )
        .unwrap();
    let running = svc
        .enqueue(
            Enqueue {
                from: Actor::User,
                to_bot_id: "asst_ava".into(),
                to_name: "Ava".into(),
                room_id: Some(room_a.id.clone()),
                kind: DeliveryKind::Mention,
                body: "进行中".into(),
                exchange_id: None,
                parent_delivery_id: None,
                sensitive: false,
                approved: true,
                attachment_ids: Vec::new(),
                attachment_sources: Vec::new(),
            },
            2,
        )
        .unwrap();
    let waiting = svc
        .enqueue(
            Enqueue {
                from: Actor::User,
                to_bot_id: "asst_ava".into(),
                to_name: "Ava".into(),
                room_id: Some(room_a.id.clone()),
                kind: DeliveryKind::Mention,
                body: "还在排队".into(),
                exchange_id: None,
                parent_delivery_id: None,
                sensitive: false,
                approved: true,
                attachment_ids: Vec::new(),
                attachment_sources: Vec::new(),
            },
            3,
        )
        .unwrap();
    let other = svc
        .enqueue(
            Enqueue {
                from: Actor::User,
                to_bot_id: "asst_bo".into(),
                to_name: "Bo".into(),
                room_id: Some(room_b.id.clone()),
                kind: DeliveryKind::Mention,
                body: "另一个房间".into(),
                exchange_id: None,
                parent_delivery_id: None,
                sensitive: false,
                approved: true,
                attachment_ids: Vec::new(),
                attachment_sources: Vec::new(),
            },
            4,
        )
        .unwrap();
    let direct = send(
        &svc,
        "asst_ava",
        "Ava",
        "asst_bo",
        "Bo",
        "私人消息",
        None,
        DeliveryKind::Peer,
        true,
        5,
    );
    let claimed = svc
        .claim_next(&HashSet::new(), "run".into(), 6)
        .unwrap()
        .unwrap();
    assert_eq!(claimed.id, running.id);
    svc.begin_send(&running.id, "run", 7).unwrap();
    svc.stop_room(&room_a.id, 8).unwrap();

    assert_eq!(
        svc.get_delivery(&waiting.id).unwrap().status,
        DeliveryStatus::Cancelled
    );
    assert_eq!(
        svc.get_delivery(&waiting.id).unwrap().error.as_deref(),
        Some(GROUP_STOP_REASON)
    );
    assert_eq!(
        svc.get_delivery(&running.id).unwrap().status,
        DeliveryStatus::Running
    );
    assert_eq!(
        svc.get_delivery(&other.id).unwrap().status,
        DeliveryStatus::Queued
    );
    assert_eq!(
        svc.get_delivery(&direct.id).unwrap().status,
        DeliveryStatus::Queued
    );
    assert_eq!(svc.room(&room_a.id).unwrap().status, RoomStatus::Stopped);
    assert_eq!(svc.room(&room_b.id).unwrap().status, RoomStatus::Active);

    let report = svc
        .finish(
            &running.id,
            "run",
            RunFinish::Succeeded {
                native_output: "已经开始的结果".into(),
                model_enqueued: false,
            },
            9,
        )
        .unwrap();
    assert!(report.follow_up.is_none());
    assert_eq!(
        svc.get_delivery(&running.id)
            .unwrap()
            .native_output
            .as_deref(),
        Some("已经开始的结果")
    );
    let next = svc
        .claim_next(&HashSet::new(), "other".into(), 10)
        .unwrap()
        .unwrap();
    assert_eq!(next.id, other.id);
}

#[test]
fn unavailable_target_is_explicit_and_not_queued() {
    let dir = tempfile::tempdir().unwrap();
    let svc = service(dir.path());
    let delivery = svc
        .record_unavailable(
            Enqueue {
                from: Actor::User,
                to_bot_id: "missing".into(),
                to_name: "missing".into(),
                room_id: None,
                kind: DeliveryKind::Peer,
                body: "在吗".into(),
                exchange_id: None,
                parent_delivery_id: None,
                sensitive: false,
                approved: true,
                attachment_ids: Vec::new(),
                attachment_sources: Vec::new(),
            },
            "Bot「missing」不可用：没有可投递的会话 conv_bot_missing".into(),
            1,
        )
        .unwrap();
    assert_eq!(delivery.status, DeliveryStatus::Unavailable);
    assert!(delivery
        .error
        .as_deref()
        .unwrap()
        .contains("conv_bot_missing"));
    assert!(delivery
        .receipts
        .iter()
        .any(|receipt| receipt.state == ReceiptState::Unavailable));
    assert!(svc
        .claim_next(&HashSet::new(), "tok".into(), 2)
        .unwrap()
        .is_none());
}

#[test]
fn restart_marks_running_interrupted_and_keeps_the_other_item_queued() {
    let dir = tempfile::tempdir().unwrap();
    let svc = service(dir.path());
    let running = svc
        .enqueue(
            Enqueue {
                from: Actor::User,
                to_bot_id: "b".into(),
                to_name: "Bo".into(),
                room_id: None,
                kind: DeliveryKind::Peer,
                body: "正在跑".into(),
                exchange_id: None,
                parent_delivery_id: None,
                sensitive: false,
                approved: true,
                attachment_ids: Vec::new(),
                attachment_sources: Vec::new(),
            },
            1,
        )
        .unwrap();
    let queued = svc
        .enqueue(
            Enqueue {
                from: Actor::User,
                to_bot_id: "c".into(),
                to_name: "Cyd".into(),
                room_id: None,
                kind: DeliveryKind::Peer,
                body: "还在队列".into(),
                exchange_id: None,
                parent_delivery_id: None,
                sensitive: false,
                approved: true,
                attachment_ids: Vec::new(),
                attachment_sources: Vec::new(),
            },
            2,
        )
        .unwrap();
    svc.claim_next(&HashSet::new(), "live".into(), 3).unwrap();
    assert!(svc.begin_send(&running.id, "live", 4).unwrap());
    drop(svc);

    let reloaded = service(dir.path());
    let interrupted = reloaded.get_delivery(&running.id).unwrap();
    assert_eq!(interrupted.status, DeliveryStatus::Interrupted);
    assert_eq!(interrupted.error.as_deref(), Some(INTERRUPT_REASON));
    assert_eq!(
        reloaded.get_delivery(&queued.id).unwrap().status,
        DeliveryStatus::Queued
    );
    let err = reloaded
        .finish(
            &running.id,
            "live",
            RunFinish::Succeeded {
                native_output: "不该重放".into(),
                model_enqueued: false,
            },
            5,
        )
        .unwrap_err();
    assert!(err.contains("未重放"));
    assert!(reloaded
        .get_delivery(&running.id)
        .unwrap()
        .native_output
        .is_none());
    let next = reloaded
        .claim_next(&HashSet::new(), "next".into(), 6)
        .unwrap()
        .unwrap();
    assert_eq!(next.id, queued.id);
}

#[test]
fn claim_token_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let svc = service(dir.path());
    let delivery = svc
        .enqueue(
            Enqueue {
                from: Actor::User,
                to_bot_id: "b".into(),
                to_name: "Bo".into(),
                room_id: None,
                kind: DeliveryKind::Peer,
                body: "一次".into(),
                exchange_id: None,
                parent_delivery_id: None,
                sensitive: false,
                approved: true,
                attachment_ids: Vec::new(),
                attachment_sources: Vec::new(),
            },
            1,
        )
        .unwrap();
    let claimed = svc
        .claim_next(&HashSet::new(), "tok".into(), 2)
        .unwrap()
        .unwrap();
    assert!(svc
        .claim_next(&HashSet::new(), "other".into(), 3)
        .unwrap()
        .is_none());
    svc.begin_send(&claimed.id, "tok", 4).unwrap();
    let wrong = svc
        .finish(
            &delivery.id,
            "other",
            RunFinish::Succeeded {
                native_output: "错的认领".into(),
                model_enqueued: false,
            },
            5,
        )
        .unwrap_err();
    assert!(wrong.contains("认领"));
    assert_eq!(
        svc.get_delivery(&delivery.id).unwrap().status,
        DeliveryStatus::Running
    );
    let report = svc
        .finish(
            &delivery.id,
            "tok",
            RunFinish::Succeeded {
                native_output: "对的结果".into(),
                model_enqueued: false,
            },
            6,
        )
        .unwrap();
    assert!(report.follow_up.is_none());
    let again = svc
        .finish(
            &delivery.id,
            "tok",
            RunFinish::Succeeded {
                native_output: "第二次".into(),
                model_enqueued: false,
            },
            7,
        )
        .unwrap();
    assert!(again.already_terminal);
    assert_eq!(
        svc.get_delivery(&delivery.id)
            .unwrap()
            .native_output
            .as_deref(),
        Some("对的结果")
    );
}

#[test]
fn pause_holds_dispatch_until_continue() {
    let dir = tempfile::tempdir().unwrap();
    let svc = service(dir.path());
    let paused = svc
        .create_room(
            "暂停",
            vec![MemberInput {
                bot_id: "asst_ava".into(),
                name: "Ava".into(),
            }],
            1,
        )
        .unwrap();
    let live = svc
        .create_room(
            "继续",
            vec![MemberInput {
                bot_id: "asst_bo".into(),
                name: "Bo".into(),
            }],
            1,
        )
        .unwrap();
    let held = svc
        .enqueue(
            Enqueue {
                from: Actor::User,
                to_bot_id: "asst_ava".into(),
                to_name: "Ava".into(),
                room_id: Some(paused.id.clone()),
                kind: DeliveryKind::Mention,
                body: "等一下".into(),
                exchange_id: None,
                parent_delivery_id: None,
                sensitive: false,
                approved: true,
                attachment_ids: Vec::new(),
                attachment_sources: Vec::new(),
            },
            2,
        )
        .unwrap();
    let moving = svc
        .enqueue(
            Enqueue {
                from: Actor::User,
                to_bot_id: "asst_bo".into(),
                to_name: "Bo".into(),
                room_id: Some(live.id.clone()),
                kind: DeliveryKind::Mention,
                body: "现在".into(),
                exchange_id: None,
                parent_delivery_id: None,
                sensitive: false,
                approved: true,
                attachment_ids: Vec::new(),
                attachment_sources: Vec::new(),
            },
            3,
        )
        .unwrap();
    svc.pause_room(&paused.id, 4).unwrap();
    let claimed = svc
        .claim_next(&HashSet::new(), "go".into(), 5)
        .unwrap()
        .unwrap();
    assert_eq!(claimed.id, moving.id);
    assert_eq!(
        svc.get_delivery(&held.id).unwrap().status,
        DeliveryStatus::Queued
    );
    svc.continue_room(&paused.id, 6).unwrap();
    svc.begin_send(&moving.id, "go", 7).unwrap();
    let next = svc
        .claim_next(&HashSet::new(), "held".into(), 8)
        .unwrap()
        .unwrap();
    assert_eq!(next.id, held.id);
}

#[test]
fn handoff_waits_for_the_user_before_it_can_dispatch() {
    let dir = tempfile::tempdir().unwrap();
    let svc = service(dir.path());
    let room = svc
        .create_room(
            "交接",
            vec![
                MemberInput {
                    bot_id: "asst_ava".into(),
                    name: "Ava".into(),
                },
                MemberInput {
                    bot_id: "asst_bo".into(),
                    name: "Bo".into(),
                },
            ],
            1,
        )
        .unwrap();
    let delivery = svc
        .enqueue(
            Enqueue {
                from: Actor::bot("asst_ava", "Ava"),
                to_bot_id: "asst_bo".into(),
                to_name: "Bo".into(),
                room_id: Some(room.id.clone()),
                kind: DeliveryKind::Handoff,
                body: "交给你".into(),
                exchange_id: None,
                parent_delivery_id: None,
                sensitive: true,
                approved: false,
                attachment_ids: Vec::new(),
                attachment_sources: Vec::new(),
            },
            2,
        )
        .unwrap();
    assert_eq!(delivery.status, DeliveryStatus::AwaitingUser);
    assert!(svc
        .claim_next(&HashSet::new(), "no".into(), 3)
        .unwrap()
        .is_none());
    let action = svc
        .snapshot()
        .pending
        .into_iter()
        .find(|action| action.delivery_id.as_deref() == Some(delivery.id.as_str()))
        .unwrap();
    assert_eq!(action.status, PendingStatus::AwaitingUser);
    svc.resolve(&action.id, true, 4).unwrap();
    assert_eq!(
        svc.get_delivery(&delivery.id).unwrap().status,
        DeliveryStatus::Queued
    );
    let claimed = svc
        .claim_next(&HashSet::new(), "yes".into(), 5)
        .unwrap()
        .unwrap();
    assert_eq!(claimed.id, delivery.id);
}

#[test]
fn removing_a_member_does_not_touch_another_room() {
    let dir = tempfile::tempdir().unwrap();
    let svc = service(dir.path());
    let room_a = svc
        .create_room(
            "甲",
            vec![
                MemberInput {
                    bot_id: "asst_ava".into(),
                    name: "Ava".into(),
                },
                MemberInput {
                    bot_id: "asst_bo".into(),
                    name: "Bo".into(),
                },
            ],
            1,
        )
        .unwrap();
    let room_b = svc
        .create_room(
            "乙",
            vec![MemberInput {
                bot_id: "asst_cyd".into(),
                name: "Cyd".into(),
            }],
            1,
        )
        .unwrap();
    let queued = svc
        .enqueue(
            Enqueue {
                from: Actor::User,
                to_bot_id: "asst_bo".into(),
                to_name: "Bo".into(),
                room_id: Some(room_a.id.clone()),
                kind: DeliveryKind::Mention,
                body: "给 Bo".into(),
                exchange_id: None,
                parent_delivery_id: None,
                sensitive: false,
                approved: true,
                attachment_ids: Vec::new(),
                attachment_sources: Vec::new(),
            },
            2,
        )
        .unwrap();
    let other = svc
        .enqueue(
            Enqueue {
                from: Actor::User,
                to_bot_id: "asst_cyd".into(),
                to_name: "Cyd".into(),
                room_id: Some(room_b.id.clone()),
                kind: DeliveryKind::Mention,
                body: "给 Cyd".into(),
                exchange_id: None,
                parent_delivery_id: None,
                sensitive: false,
                approved: true,
                attachment_ids: Vec::new(),
                attachment_sources: Vec::new(),
            },
            3,
        )
        .unwrap();
    let action = svc
        .request_member_change(&room_a.id, PendingKind::RemoveMember, "asst_bo", "Bo", 4)
        .unwrap();
    svc.resolve(&action.id, true, 5).unwrap();
    assert!(svc
        .room(&room_a.id)
        .unwrap()
        .members
        .iter()
        .all(|member| member.bot_id != "asst_bo"));
    assert_eq!(svc.room(&room_b.id).unwrap().members[0].bot_id, "asst_cyd");
    assert_eq!(
        svc.get_delivery(&queued.id).unwrap().status,
        DeliveryStatus::Cancelled
    );
    assert_eq!(
        svc.get_delivery(&other.id).unwrap().status,
        DeliveryStatus::Queued
    );
}

#[test]
fn unreadable_file_is_not_overwritten() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("collaboration.json");
    std::fs::create_dir(&path).unwrap();
    std::fs::write(path.join("keep.txt"), "preserve").unwrap();
    let svc = service(dir.path());
    let err = svc
        .enqueue(
            Enqueue {
                from: Actor::User,
                to_bot_id: "b".into(),
                to_name: "Bo".into(),
                room_id: None,
                kind: DeliveryKind::Peer,
                body: "不要覆盖".into(),
                exchange_id: None,
                parent_delivery_id: None,
                sensitive: false,
                approved: true,
                attachment_ids: Vec::new(),
                attachment_sources: Vec::new(),
            },
            1,
        )
        .unwrap_err();
    assert!(err.contains("无法读取"));
    assert_eq!(
        std::fs::read_to_string(path.join("keep.txt")).unwrap(),
        "preserve"
    );
}

#[test]
fn concurrent_enqueues_all_persist() {
    let dir = tempfile::tempdir().unwrap();
    let svc = Arc::new(service(dir.path()));
    std::thread::scope(|scope| {
        for index in 0..8 {
            let svc = Arc::clone(&svc);
            scope.spawn(move || {
                svc.enqueue(
                    Enqueue {
                        from: Actor::User,
                        to_bot_id: format!("bot{index}"),
                        to_name: format!("Bot{index}"),
                        room_id: None,
                        kind: DeliveryKind::Peer,
                        body: format!("消息{index}"),
                        exchange_id: None,
                        parent_delivery_id: None,
                        sensitive: false,
                        approved: true,
                        attachment_ids: Vec::new(),
                        attachment_sources: Vec::new(),
                    },
                    index as i64,
                )
                .unwrap();
            });
        }
    });
    assert_eq!(svc.snapshot().deliveries.len(), 8);
    let reloaded = service(dir.path());
    assert_eq!(reloaded.snapshot().deliveries.len(), 8);
}

#[test]
fn native_classifier_refuses_a_fabricated_reply() {
    let missing = classify_native_reply("user", "请看一下草案", Some("completed"), false);
    match missing {
        RunFinish::Failed { message } => assert!(message.contains(NO_FABRICATED_REPLY)),
        other => panic!("unexpected {other:?}"),
    }
    let empty = classify_native_reply("assistant", "  ", Some("completed"), false);
    match empty {
        RunFinish::Failed { message } => assert!(message.contains(NO_FABRICATED_REPLY)),
        other => panic!("unexpected {other:?}"),
    }
    let cancelled = classify_native_reply("assistant", "半截", Some("cancelled"), false);
    assert!(matches!(cancelled, RunFinish::Cancelled { .. }));
    let ok = classify_native_reply("assistant", "草案可以", Some("completed"), false);
    match ok {
        RunFinish::Succeeded {
            native_output,
            model_enqueued,
        } => {
            assert_eq!(native_output, "草案可以");
            assert!(!model_enqueued);
        }
        other => panic!("unexpected {other:?}"),
    }
    let spent = classify_native_reply("assistant", "  ", Some("completed"), true);
    match spent {
        RunFinish::Succeeded {
            native_output,
            model_enqueued,
        } => {
            assert!(native_output.is_empty());
            assert!(model_enqueued);
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn earlier_coordination_tool_spends_the_reply_budget() {
    let continued = conversation_with_messages(serde_json::json!([
        {"id":"u","role":"user","content":"[kivio-peer dlv_1]","timestamp":1},
        {"id":"tool","role":"assistant","content":"","timestamp":2,"stream_outcome":"completed",
            "tool_calls":[{"id":"call","name":"bot_peer_send","status":"success","arguments":"{}"}]},
        {"id":"final","role":"assistant","content":"收到","timestamp":3,"stream_outcome":"completed"}
    ]));
    match super::runtime::observe_conversation(&continued) {
        RunFinish::Succeeded {
            native_output,
            model_enqueued,
        } => {
            assert_eq!(native_output, "收到");
            assert!(
                model_enqueued,
                "the earlier tool already continued the exchange"
            );
        }
        other => panic!("unexpected {other:?}"),
    }

    let previous_turn = conversation_with_messages(serde_json::json!([
        {"id":"u1","role":"user","content":"上一轮","timestamp":1},
        {"id":"old","role":"assistant","content":"","timestamp":2,"stream_outcome":"completed",
            "tool_calls":[{"id":"call","name":"bot_room_mention","status":"success","arguments":"{}"}]},
        {"id":"u2","role":"user","content":"[kivio-peer dlv_2]","timestamp":3},
        {"id":"final","role":"assistant","content":"收到","timestamp":4,"stream_outcome":"completed"}
    ]));
    match super::runtime::observe_conversation(&previous_turn) {
        RunFinish::Succeeded {
            native_output,
            model_enqueued,
        } => {
            assert_eq!(native_output, "收到");
            assert!(!model_enqueued, "a previous turn must not spend this hop");
        }
        other => panic!("unexpected {other:?}"),
    }
}

fn conversation_with_messages(messages: serde_json::Value) -> crate::chat::types::Conversation {
    serde_json::from_value(serde_json::json!({
        "id": "conv_bot_asst_bo",
        "title": "Bo",
        "provider_id": "loopback",
        "model": "smoke",
        "messages": messages,
        "created_at": 1,
        "updated_at": 1
    }))
    .unwrap()
}

#[test]
fn room_member_thread_uses_the_core_helper() {
    let dir = tempfile::tempdir().unwrap();
    let svc = service(dir.path());
    let rejected = svc
        .create_room(
            "坏",
            vec![MemberInput {
                bot_id: "ava".into(),
                name: "Ava".into(),
            }],
            1,
        )
        .unwrap_err();
    assert!(svc.snapshot().rooms.is_empty());
    assert!(!rejected.contains("conv_bot_ava"));
    let room = svc
        .create_room(
            "好",
            vec![MemberInput {
                bot_id: "asst_ada".into(),
                name: "Ada".into(),
            }],
            2,
        )
        .unwrap();
    assert_eq!(
        room.members[0].conversation_id,
        crate::chat::bot_profile::member_conversation_id("asst_ada", &room.id).unwrap()
    );
    assert_eq!(
        super::member_room_id(&room.members[0].conversation_id),
        Some(room.id.as_str())
    );
}

#[test]
fn unaddressed_broadcast_is_bounded_and_does_not_chain() {
    let dir = tempfile::tempdir().unwrap();
    let svc = service(dir.path());
    let members = ["asst_a", "asst_b", "asst_c", "asst_d"]
        .into_iter()
        .map(|id| MemberInput {
            bot_id: id.into(),
            name: id.into(),
        })
        .collect();
    let room = svc.create_room("群", members, 1).unwrap();
    let receipt = svc
        .broadcast(
            Actor::User,
            &room.id,
            "没有点名".into(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            2,
        )
        .unwrap();
    assert_eq!(receipt.bound, 8);
    assert_eq!(receipt.delivery_ids.len(), 4);
    assert!(receipt.skipped_bot_ids.is_empty());
    let deliveries = svc.snapshot().deliveries;
    assert!(deliveries
        .iter()
        .all(|delivery| delivery.kind == DeliveryKind::Broadcast));
    assert!(deliveries
        .iter()
        .all(|delivery| delivery.exchange_id == deliveries[0].exchange_id));
    let token = "cast".to_string();
    let claimed = svc
        .claim_next(&HashSet::new(), token.clone(), 3)
        .unwrap()
        .unwrap();
    svc.begin_send(&claimed.id, &token, 4).unwrap();
    let report = svc
        .finish(
            &claimed.id,
            &token,
            RunFinish::Succeeded {
                native_output: "只记在房间里".into(),
                model_enqueued: false,
            },
            5,
        )
        .unwrap();
    assert!(report.follow_up.is_none());
    assert_eq!(svc.snapshot().deliveries.len(), 4);
}

#[test]
fn rename_rejects_an_empty_or_oversized_title() {
    let dir = tempfile::tempdir().unwrap();
    let svc = service(dir.path());
    let room = svc
        .create_room(
            "原名",
            vec![MemberInput {
                bot_id: "asst_ada".into(),
                name: "Ada".into(),
            }],
            1,
        )
        .unwrap();
    assert!(svc.rename_room(&room.id, "  ", 2).is_err());
    assert!(svc.rename_room(&room.id, &"名".repeat(81), 3).is_err());
    let renamed = svc.rename_room(&room.id, "  新房间  ", 4).unwrap();
    assert_eq!(renamed.title, "新房间");
}

#[test]
fn discard_room_removes_a_room_with_no_delivery() {
    let dir = tempfile::tempdir().unwrap();
    let svc = service(dir.path());
    let room = svc
        .create_room(
            "未完成",
            vec![MemberInput {
                bot_id: "asst_ada".into(),
                name: "Ada".into(),
            }],
            1,
        )
        .unwrap();
    svc.discard_room(&room.id).unwrap();
    assert!(svc.snapshot().rooms.is_empty());
    let kept = svc
        .create_room(
            "已投递",
            vec![MemberInput {
                bot_id: "asst_ada".into(),
                name: "Ada".into(),
            }],
            2,
        )
        .unwrap();
    svc.broadcast(
        Actor::User,
        &kept.id,
        "在".into(),
        Vec::new(),
        Vec::new(),
        None,
        None,
        3,
    )
    .unwrap();
    assert!(svc.discard_room(&kept.id).is_err());
    assert_eq!(svc.snapshot().rooms.len(), 1);
}

#[test]
fn different_bots_are_claimed_without_waiting_for_each_other() {
    let dir = tempfile::tempdir().unwrap();
    let svc = service(dir.path());
    send(
        &svc,
        "asst_ava",
        "Ava",
        "asst_bo",
        "Bo",
        "给 Bo",
        None,
        DeliveryKind::Peer,
        true,
        1,
    );
    send(
        &svc,
        "asst_ava",
        "Ava",
        "asst_cyd",
        "Cyd",
        "给 Cyd",
        None,
        DeliveryKind::Peer,
        true,
        2,
    );
    let first = svc
        .claim_next(&HashSet::new(), "bo".into(), 3)
        .unwrap()
        .unwrap();
    let second = svc
        .claim_next(&HashSet::new(), "cyd".into(), 4)
        .unwrap()
        .unwrap();
    assert_ne!(first.to_bot_id, second.to_bot_id);
    assert_eq!(first.status, DeliveryStatus::Claimed);
    assert_eq!(second.status, DeliveryStatus::Claimed);
    assert!(MAX_PARALLEL_DISPATCH >= 2);
}

#[test]
fn model_continuation_stays_on_the_inbound_exchange() {
    let dir = tempfile::tempdir().unwrap();
    let svc = service(dir.path());
    let room = svc
        .create_room(
            "群",
            vec![
                MemberInput {
                    bot_id: "asst_ava".into(),
                    name: "Ava".into(),
                },
                MemberInput {
                    bot_id: "asst_bo".into(),
                    name: "Bo".into(),
                },
                MemberInput {
                    bot_id: "asst_cyd".into(),
                    name: "Cyd".into(),
                },
            ],
            1,
        )
        .unwrap();
    let inbound = send(
        &svc,
        "asst_ava",
        "Ava",
        "asst_bo",
        "Bo",
        "先看这个",
        Some(room.id.clone()),
        DeliveryKind::Mention,
        true,
        2,
    );
    let claimed = svc
        .claim_next(&HashSet::new(), "run".into(), 3)
        .unwrap()
        .unwrap();
    assert!(svc.begin_send(&claimed.id, "run", 4).unwrap());
    assert_eq!(
        svc.active_inbound(&inbound.to_conversation_id).unwrap().id,
        inbound.id
    );
    let receipt = svc
        .broadcast(
            Actor::bot("asst_bo", "Bo"),
            &room.id,
            "同一交流".into(),
            Vec::new(),
            Vec::new(),
            Some(inbound.exchange_id.clone()),
            Some(inbound.id.clone()),
            5,
        )
        .unwrap();
    assert!(receipt.delivery_ids.len() >= 2);
    for id in &receipt.delivery_ids {
        let delivery = svc.get_delivery(id).unwrap();
        assert_eq!(delivery.kind, DeliveryKind::Broadcast);
        assert_eq!(delivery.exchange_id, inbound.exchange_id);
        assert_eq!(
            delivery.parent_delivery_id.as_deref(),
            Some(inbound.id.as_str())
        );
        assert_eq!(delivery.depth, 1);
    }
    let continued = svc.get_delivery(&receipt.delivery_ids[0]).unwrap();
    let reset = svc
        .enqueue(
            Enqueue {
                from: Actor::bot("asst_bo", "Bo"),
                to_bot_id: "asst_ava".into(),
                to_name: "Ava".into(),
                room_id: Some(room.id.clone()),
                kind: DeliveryKind::Peer,
                body: "另开一次".into(),
                exchange_id: Some("ex_other".into()),
                parent_delivery_id: Some(inbound.id.clone()),
                sensitive: false,
                approved: true,
                attachment_ids: Vec::new(),
                attachment_sources: Vec::new(),
            },
            6,
        )
        .unwrap_err();
    assert!(reset.contains("同一次交流"));
    let hop = svc
        .enqueue(
            Enqueue {
                from: Actor::bot("asst_cyd", "Cyd"),
                to_bot_id: "asst_ava".into(),
                to_name: "Ava".into(),
                room_id: Some(room.id.clone()),
                kind: DeliveryKind::Handoff,
                body: "再交接".into(),
                exchange_id: Some(continued.exchange_id.clone()),
                parent_delivery_id: Some(continued.id.clone()),
                sensitive: true,
                approved: false,
                attachment_ids: Vec::new(),
                attachment_sources: Vec::new(),
            },
            8,
        )
        .unwrap_err();
    assert!(hop.contains("协作轮次已到上限"));
}

#[test]
fn running_is_durable_before_the_recipient_finishes() {
    let dir = tempfile::tempdir().unwrap();
    let svc = service(dir.path());
    let delivery = send(
        &svc,
        "asst_ava",
        "Ava",
        "asst_bo",
        "Bo",
        "开始",
        None,
        DeliveryKind::Peer,
        true,
        1,
    );
    let claimed = svc
        .claim_next(&HashSet::new(), "tok".into(), 2)
        .unwrap()
        .unwrap();
    assert!(svc.begin_send(&claimed.id, "tok", 3).unwrap());
    let stored: super::types::Delivery = serde_json::from_str(
        &std::fs::read_to_string(delivery_record_path(dir.path(), &delivery.id)).unwrap(),
    )
    .unwrap();
    assert_eq!(stored.status, DeliveryStatus::Running);
    assert!(stored.admitted);
}

#[test]
fn attachment_resolution_uses_the_stored_file_and_not_the_id() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("notes.txt"), "hello attachment").unwrap();
    let file = crate::chat::types::Attachment {
        id: "att_notes".into(),
        attachment_type: "file".into(),
        name: "notes.txt".into(),
        path: "notes.txt".into(),
        content: None,
    };
    match super::flow::resolve_stored_attachment(&file, dir.path()).unwrap() {
        super::flow::ResolvedAttachment::File(path) => {
            assert_eq!(std::fs::read_to_string(path).unwrap(), "hello attachment");
        }
        super::flow::ResolvedAttachment::Text { .. } => panic!("expected the stored file"),
    }
    let memory = crate::chat::types::Attachment {
        id: "att_mem".into(),
        attachment_type: "file".into(),
        name: "pasted.txt".into(),
        path: "memory://1".into(),
        content: Some("pasted body".into()),
    };
    match super::flow::resolve_stored_attachment(&memory, dir.path()).unwrap() {
        super::flow::ResolvedAttachment::Text { name, content } => {
            assert_eq!(name, "pasted.txt");
            assert_eq!(content, "pasted body");
        }
        super::flow::ResolvedAttachment::File(_) => panic!("expected the memory text"),
    }
    let escape = crate::chat::types::Attachment {
        id: "att_escape".into(),
        attachment_type: "file".into(),
        name: "notes.txt".into(),
        path: "../notes.txt".into(),
        content: None,
    };
    assert!(super::flow::resolve_stored_attachment(&escape, dir.path()).is_err());
}

fn peer_delivery(id: &str, to: &str, body: &str) -> super::types::Delivery {
    serde_json::from_value(serde_json::json!({
        "id": id,
        "exchangeId": "ex_1",
        "entryId": "entry_1",
        "roomCycle": 1,
        "from": {"kind": "bot", "id": "asst_ada", "name": "Ada"},
        "toBotId": "asst_bo",
        "toName": "Bo",
        "toConversationId": to,
        "kind": "peer",
        "body": body,
        "depth": 0,
        "status": "succeeded",
        "createdAt": 1,
        "updatedAt": 1
    }))
    .unwrap()
}

fn user_parts(messages: &[serde_json::Value]) -> Vec<String> {
    let user = messages
        .iter()
        .find(|message| message["role"] == "user")
        .unwrap();
    match &user["content"] {
        serde_json::Value::String(text) => vec![text.clone()],
        serde_json::Value::Array(parts) => parts
            .iter()
            .filter_map(|part| part["text"].as_str().map(str::to_string))
            .collect(),
        other => panic!("unexpected user content {other}"),
    }
}

#[test]
fn peer_replay_keeps_ordinary_text_one_envelope_and_its_own_delivery() {
    let conversation_id = "conv_bot_asst_bo";
    let delivery = peer_delivery("dlv_old", conversation_id, "旧正文");
    let envelope = super::peer_user_message(&delivery);
    assert_eq!(
        envelope,
        super::peer_envelope(&super::types::PeerMessage::from(&delivery), "旧正文")
    );

    let prefix = format!("[kivio-peer {}]\n这不是完整信封", delivery.id);
    let mut ordinary = conversation_with_messages(serde_json::json!([
        {"id":"u","role":"user","content": prefix, "timestamp": 1}
    ]));
    super::project_trusted_legacy_peers(&mut ordinary, std::slice::from_ref(&delivery));
    assert_eq!(ordinary.messages[0].content, prefix);
    assert!(ordinary.messages[0].peer.is_none());
    let ordinary_replay = crate::chat::commands::context::build_chat_api_messages_with_deliveries(
        None,
        "system",
        &ordinary,
        None,
        None,
        &[],
        false,
        std::slice::from_ref(&delivery),
    )
    .unwrap();
    assert_eq!(user_parts(&ordinary_replay), vec![prefix]);

    let foreign = peer_delivery("dlv_foreign", "conv_other", "别人的正文");
    let foreign_envelope = super::peer_user_message(&foreign);
    let mut leaked = conversation_with_messages(serde_json::json!([
        {"id":"u","role":"user","content": foreign_envelope, "timestamp": 1}
    ]));
    super::project_trusted_legacy_peers(&mut leaked, std::slice::from_ref(&foreign));
    assert_eq!(leaked.messages[0].content, foreign_envelope);
    assert!(leaked.messages[0].peer.is_none());
    let leaked_replay = crate::chat::commands::context::build_chat_api_messages_with_deliveries(
        None,
        "system",
        &leaked,
        None,
        None,
        &[],
        false,
        std::slice::from_ref(&foreign),
    )
    .unwrap();
    assert_eq!(user_parts(&leaked_replay), vec![foreign_envelope.clone()]);

    let mut duplicated = conversation_with_messages(serde_json::json!([
        {"id":"u","role":"user","content": envelope, "timestamp": 1}
    ]));
    let copies = vec![delivery.clone(), delivery.clone()];
    super::project_trusted_legacy_peers(&mut duplicated, &copies);
    assert_eq!(duplicated.messages[0].content, envelope);
    assert!(duplicated.messages[0].peer.is_none());

    let mut legacy = conversation_with_messages(serde_json::json!([
        {"id":"u","role":"user","content": envelope, "timestamp": 1}
    ]));
    legacy.messages[0].attachments = vec![crate::chat::types::Attachment {
        id: "att_clip".into(),
        attachment_type: "video".into(),
        name: "clip.mp4".into(),
        path: "clip.mp4".into(),
        content: None,
    }];
    let before = crate::chat::commands::context::build_chat_api_messages_with_deliveries(
        None,
        "system",
        &legacy,
        None,
        None,
        &[],
        false,
        std::slice::from_ref(&delivery),
    )
    .unwrap();
    let before_parts = user_parts(&before);
    assert_eq!(
        before_parts
            .iter()
            .filter(|part| part.contains("[Video attachment:"))
            .count(),
        1
    );
    assert_eq!(
        before_parts
            .iter()
            .find(|part| part.starts_with("[kivio-peer"))
            .map(String::as_str),
        Some(envelope.as_str())
    );

    super::project_trusted_legacy_peers(&mut legacy, std::slice::from_ref(&delivery));
    assert_eq!(legacy.messages[0].content, "旧正文");
    assert_eq!(legacy.messages[0].attachments.len(), 1);
    assert_eq!(
        legacy.messages[0]
            .peer
            .as_ref()
            .map(|peer| peer.delivery_id.as_str()),
        Some("dlv_old")
    );
    let after = crate::chat::commands::context::build_chat_api_messages_with_deliveries(
        None,
        "system",
        &legacy,
        None,
        None,
        &[],
        false,
        &[],
    )
    .unwrap();
    assert_eq!(user_parts(&before), user_parts(&after));

    let composed = "旧正文\n\n[Attachment: spec.txt]";
    let current = crate::chat::commands::context::build_chat_api_messages_with_deliveries(
        None,
        "system",
        &legacy,
        Some(0),
        Some(composed),
        &[],
        false,
        &[],
    )
    .unwrap();
    let replay = crate::chat::commands::context::build_chat_api_messages_with_deliveries(
        None,
        "system",
        &legacy,
        None,
        None,
        &[],
        false,
        &[],
    )
    .unwrap();
    let current_parts = user_parts(&current);
    let replay_parts = user_parts(&replay);
    let current_envelope = current_parts
        .iter()
        .find(|part| part.starts_with("[kivio-peer"))
        .unwrap();
    let replay_envelope = replay_parts
        .iter()
        .find(|part| part.starts_with("[kivio-peer"))
        .unwrap();
    let peer = legacy.messages[0].peer.clone().unwrap();
    assert_eq!(current_envelope, &super::peer_envelope(&peer, composed));
    assert_eq!(replay_envelope, &super::peer_envelope(&peer, "旧正文"));
    assert_eq!(
        current_envelope.split_once("\n\n").unwrap().0,
        replay_envelope.split_once("\n\n").unwrap().0
    );
    assert_eq!(current_envelope.matches(composed).count(), 1);
    assert_eq!(replay_envelope.matches("旧正文").count(), 1);
    assert!(!replay_envelope.contains("[Attachment: spec.txt]"));
    assert_eq!(
        current_parts
            .iter()
            .filter(|part| part.contains("[Video attachment:"))
            .count(),
        1
    );
    assert_eq!(
        replay_parts
            .iter()
            .filter(|part| part.contains("[Video attachment:"))
            .count(),
        1
    );
}
