mod support;

use kivio_agent_sdk::{
    agent::{
        content::ContentKind,
        control::{InputAdmission, StopAdmission},
        events::{AgentEvent, AgentEventKind, AgentRunOutcome, EventDelivery},
        run_agent_loop,
        session::NoSessionPolicy,
        types::AgentRunStatus,
        RuntimeMessage, SteeringMessage,
    },
    model::{
        openai::{OpenAiChatConfig, OpenAiChatProvider},
        ModelRole,
    },
};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use support::{config, handle, initial_history, Control, EchoTool, Host};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let mode = args.first().map(String::as_str).unwrap_or("minimal");
    if !matches!(mode, "minimal" | "cancel" | "inputs" | "continue")
        || args.len() > 2
        || args.get(1).is_some_and(|arg| arg != "--live")
    {
        return Err(std::io::Error::other(
            "usage: agent [minimal|cancel|inputs|continue] [--live]",
        )
        .into());
    }
    let live = args.get(1).is_some();
    let mut server = if live {
        None
    } else {
        Some(support::http::Server::start(mode == "inputs", mode == "continue").await?)
    };
    let (endpoint, model, api_key) = if let Some(server) = &server {
        (server.endpoint.clone(), "example-model".into(), None)
    } else {
        (
            std::env::var("KIVIO_SDK_BASE_URL")?.parse()?,
            std::env::var("KIVIO_SDK_MODEL")?,
            std::env::var("KIVIO_SDK_API_KEY").ok(),
        )
    };
    let mut transport = OpenAiChatConfig::new(endpoint);
    transport.api_key = api_key;
    let mut client = reqwest::Client::builder().timeout(Duration::from_secs(60));
    if !live {
        client = client.no_proxy();
    }
    let provider = OpenAiChatProvider::new(client.build()?, transport);
    let control = Control::default();
    let run = handle(&control);
    let executor = EchoTool::new(true);
    let submitted = AtomicBool::new(false);
    let sink = |event: &AgentEvent<'_>| {
        print_event(event);
        if mode == "inputs"
            && matches!(event.kind, AgentEventKind::ToolEnded { .. })
            && !submitted.swap(true, Ordering::SeqCst)
        {
            assert_eq!(
                run.steer(
                    SteeringMessage::new("steer-1".into(), "Keep the answer short.").unwrap()
                ),
                InputAdmission::Accepted
            );
            assert_eq!(
                run.follow_up(
                    SteeringMessage::new("follow-1".into(), "Now summarize what echo returned.")
                        .unwrap()
                ),
                InputAdmission::Accepted
            );
            println!("steer and follow-up accepted into the host mailbox");
        }
        if mode == "cancel"
            && matches!(&event.kind, AgentEventKind::Content { delta, .. } if !delta.is_empty())
            && !submitted.swap(true, Ordering::SeqCst)
        {
            assert_eq!(run.stop(), StopAdmission::Requested);
        }
        EventDelivery::Accepted
    };
    let mut run_config = config(&provider, initial_history());
    run_config.model = model.clone();
    let result = run_agent_loop(
        run_config,
        &Host,
        &executor,
        &mut NoSessionPolicy,
        &sink,
        run.clone(),
    )
    .await
    .map_err(std::io::Error::other)?;
    println!(
        "status: {:?}; echo calls: {}",
        result.status,
        executor.call_count()
    );
    assert!(!run.active());
    assert_eq!(run.stop(), StopAdmission::Inactive);
    if mode == "cancel" {
        assert_eq!(result.status, AgentRunStatus::Cancelled);
    } else {
        assert_eq!(result.status, AgentRunStatus::Completed);
        println!("{}", result.content);
    }
    if !live {
        assert_eq!(executor.call_count(), 1);
    }
    if mode == "inputs" {
        for id in ["steer-1", "follow-1"] {
            assert_eq!(
                result.segments.iter().filter(|block| {
                    matches!(&block.kind, ContentKind::Input { message, .. } if message.id == id)
                }).count(),
                1,
                "each admitted input must actually be consumed once"
            );
        }
    }
    if mode == "continue" {
        let calls_before = executor.call_count();
        let mut history = result.history;
        history.push(RuntimeMessage::text(
            ModelRole::User,
            "Summarize the previous result without calling any tools.",
        ));
        let next_control = Control::default();
        let mut next_config = config(&provider, history);
        next_config.model = model;
        let continued = run_agent_loop(
            next_config,
            &Host,
            &executor,
            &mut NoSessionPolicy,
            &print_event,
            handle(&next_control),
        )
        .await
        .map_err(std::io::Error::other)?;
        assert_eq!(continued.status, AgentRunStatus::Completed);
        if !live {
            assert_eq!(executor.call_count(), calls_before);
        }
        println!("continued: {}", continued.content);
    }
    if let Some(server) = &mut server {
        server.finish().await?;
    }
    Ok(())
}

fn print_event(event: &AgentEvent<'_>) -> EventDelivery {
    let fact = match &event.kind {
        AgentEventKind::RunStarted => "run started".into(),
        AgentEventKind::RunEnded { outcome } => format!(
            "run ended: {}",
            match outcome {
                AgentRunOutcome::Completed => "completed",
                AgentRunOutcome::Recovered => "recovered",
                AgentRunOutcome::CompactionBlocked => "compaction blocked",
                AgentRunOutcome::Cancelled => "cancelled",
                AgentRunOutcome::Failed(_) => "failed",
                AgentRunOutcome::Dropped => "dropped",
            }
        ),
        AgentEventKind::ToolStarted { name, .. } => {
            format!(
                "tool started: {}",
                name.chars().take(64).collect::<String>()
            )
        }
        AgentEventKind::ToolEnded { cancelled, .. } => {
            format!("tool ended; cancelled: {cancelled}")
        }
        AgentEventKind::ToolUpdated(record) => format!("tool status: {:?}", record.status),
        AgentEventKind::Content { delta, block, .. } => match block.map(|block| &block.kind) {
            Some(ContentKind::Input { kind, .. }) => format!("input consumed: {kind:?}"),
            _ => format!("content delta: {} characters", delta.chars().count()),
        },
        _ => return EventDelivery::Accepted,
    };
    println!(
        "invocation {} event {}: {fact}",
        event.invocation_id, event.sequence
    );
    EventDelivery::Accepted
}
