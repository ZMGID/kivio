use std::{sync::Arc, time::Duration};

use kivio_agent_sdk::{
    agent::{AgentRunError, AgentSession, AgentSessionConfig, SessionSnapshot},
    model::openai::{OpenAiChatConfig, OpenAiChatProvider},
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut transport = OpenAiChatConfig::new(std::env::var("KIVIO_SDK_BASE_URL")?.parse()?);
    transport.api_key = std::env::var("KIVIO_SDK_API_KEY").ok();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()?;
    let runtime = Arc::new(OpenAiChatProvider::new(client, transport));
    let session = AgentSession::new(AgentSessionConfig::new(
        runtime.clone(),
        std::env::var("KIVIO_SDK_MODEL")?,
    ));
    println!(
        "{}",
        session
            .prompt("Explain what an agent session owns in one sentence.")
            .await?
            .content
    );
    let bytes = serde_json::to_vec(&session.snapshot().map_err(AgentRunError::Admission)?)?;
    let saved: SessionSnapshot = serde_json::from_slice(&bytes)?;
    let restored = AgentSession::new(AgentSessionConfig::from_snapshot(runtime, saved)?);
    println!(
        "{}",
        restored.prompt("Now give one example.").await?.content
    );
    Ok(())
}
