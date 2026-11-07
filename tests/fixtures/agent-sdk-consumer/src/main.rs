use kivio_agent_sdk::{
    agent::{run_agent_loop, session::NoSessionPolicy, types::AgentRunStatus},
    model::openai::{OpenAiChatConfig, OpenAiChatProvider},
};
use kivio_agent_sdk_consumer::{config, handle, initial_history, Control, EchoTool, Events, Host};
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut transport = OpenAiChatConfig::new(std::env::var("KIVIO_SDK_BASE_URL")?.parse()?);
    transport.api_key = std::env::var("KIVIO_SDK_API_KEY").ok();
    let model = std::env::var("KIVIO_SDK_MODEL")?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()?;
    let provider = OpenAiChatProvider::new(client, transport);
    let control = Control::default();
    let events = Events::default();
    let mut run_config = config(&provider, initial_history());
    run_config.model = model;
    let executor = EchoTool::new(true);
    let result = run_agent_loop(
        run_config,
        &Host,
        &executor,
        &mut NoSessionPolicy,
        &events,
        handle(&control),
    )
    .await
    .map_err(std::io::Error::other)?;
    println!("{}", result.content);
    println!(
        "status: {:?}; tool_calls: {}; events: {}",
        result.status,
        executor.call_count(),
        events.0.lock().unwrap().len()
    );
    if !matches!(
        result.status,
        AgentRunStatus::Completed | AgentRunStatus::Recovered
    ) {
        return Err(std::io::Error::other(format!("SDK run ended: {:?}", result.status)).into());
    }
    Ok(())
}
