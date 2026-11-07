use std::{sync::Arc, time::Duration};

use kivio_agent_sdk::{
    agent::{AgentSession, AgentSessionConfig},
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
        runtime,
        std::env::var("KIVIO_SDK_MODEL")?,
    ));
    println!(
        "{}",
        session
            .prompt("Explain what an agent session owns in one sentence.")
            .await?
            .content
    );
    println!("{}", session.prompt("Now give one example.").await?.content);
    Ok(())
}
