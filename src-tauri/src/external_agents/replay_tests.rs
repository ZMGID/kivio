use serde::Deserialize;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::time::{timeout, Duration};

const MAX_FRAME: usize = 64 * 1024;
const MAX_TRANSCRIPT: usize = 512 * 1024;
pub(crate) const DEADLINE: Duration = Duration::from_secs(5);

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub(crate) enum Step {
    #[serde(rename = "expect_outbound")]
    Expect { frame: Value },
    #[serde(rename = "emit_inbound")]
    Emit { frame: Value },
}

pub(crate) fn fixture(name: &str) -> Vec<Step> {
    let source = match name {
        "codex-real-turn" => include_str!("fixtures/codex-real-turn.ndjson"),
        "pi-real-turn" => include_str!("fixtures/pi-real-turn.ndjson"),
        "acp-adapted-grok-turn" => include_str!("fixtures/acp-adapted-grok-turn.ndjson"),
        "claude-recorded-approval" => include_str!("fixtures/claude-recorded-approval.ndjson"),
        "claude-disconnect" => include_str!("fixtures/claude-disconnect.ndjson"),
        "dsh-two-turns" => include_str!("fixtures/dsh-two-turns.ndjson"),
        "dsh-disconnect" => include_str!("fixtures/dsh-disconnect.ndjson"),
        "dsh-stop-next" => include_str!("fixtures/dsh-stop-next.ndjson"),
        "antigravity-steps" => include_str!("fixtures/antigravity-steps.ndjson"),
        _ => panic!("unknown replay fixture {name}"),
    };
    load(source).expect("valid bounded replay fixture")
}

fn load(source: &str) -> Result<Vec<Step>, String> {
    if source.len() > MAX_TRANSCRIPT {
        return Err("transcript exceeds replay limit".into());
    }
    let mut lines = source.lines();
    let header = lines.next().ok_or("missing transcript header")?;
    if header.len() > MAX_FRAME {
        return Err("header exceeds replay frame limit".into());
    }
    let header: Value = serde_json::from_str(header).map_err(|e| e.to_string())?;
    if header["type"] != "transcript_start"
        || ["protocol", "version", "classification"]
            .iter()
            .any(|key| header[*key].as_str().is_none_or(str::is_empty))
    {
        return Err("missing replay provenance".into());
    }
    lines
        .take(1025)
        .enumerate()
        .map(|(index, line)| {
            if index >= 1024 || line.len() > MAX_FRAME {
                return Err("transcript exceeds replay frame limit".into());
            }
            serde_json::from_str(line).map_err(|e| e.to_string())
        })
        .collect()
}

fn matches_subset(actual: &Value, expected: &Value) -> bool {
    match expected {
        Value::Object(fields) => actual.as_object().is_some_and(|actual| {
            fields.iter().all(|(key, expected)| {
                actual
                    .get(key)
                    .is_some_and(|actual| matches_subset(actual, expected))
            })
        }),
        _ => actual == expected,
    }
}

pub(crate) async fn play<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    name: &str,
    input: R,
    mut output: W,
) {
    let mut input = BufReader::new(input).lines();
    for step in fixture(name) {
        match step {
            Step::Expect { frame } => {
                let line = timeout(DEADLINE, input.next_line())
                    .await
                    .expect("replay outbound deadline")
                    .expect("read replay outbound")
                    .expect("missing replay outbound");
                assert!(line.len() <= MAX_FRAME);
                let actual: Value = serde_json::from_str(&line).unwrap();
                assert!(matches_subset(&actual, &frame), "{actual} != {frame}");
            }
            Step::Emit { frame } => {
                output
                    .write_all(format!("{frame}\n").as_bytes())
                    .await
                    .unwrap();
            }
        }
    }
    output.shutdown().await.unwrap();
}

pub(crate) async fn ready<R: tokio::io::AsyncBufRead + Unpin>(reader: &mut tokio::io::Lines<R>) {
    timeout(DEADLINE, async {
        loop {
            let line = reader.next_line().await.unwrap().expect("peer ready");
            if line.ends_with("KIVIO_REPLAY_READY") {
                break;
            }
        }
    })
    .await
    .expect("replay peer startup deadline");
}

pub(crate) fn pipe_peer(
    name: &str,
    receipt: &std::path::Path,
) -> crate::external_agents::spawn::AgentChild {
    use crate::proc::NoConsoleWindow;
    crate::external_agents::spawn::AgentChild::spawn(
        tokio::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "external_agents::replay_tests::transcript_pipe_peer",
                "--nocapture",
            ])
            .env("KIVIO_REPLAY_FIXTURE", name)
            .env("KIVIO_REPLAY_RECEIPT", receipt)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .no_console_window()
            .kill_on_drop(true),
    )
    .unwrap()
}

#[test]
fn transcript_pipe_peer() {
    let Ok(name) = std::env::var("KIVIO_REPLAY_FIXTURE") else {
        return;
    };
    use std::io::{BufRead, Read, Write};
    let mut input = std::io::BufReader::new(std::io::stdin().lock());
    println!("KIVIO_REPLAY_READY");
    std::io::stdout().flush().unwrap();
    for step in fixture(&name) {
        match step {
            Step::Expect { frame } => {
                let mut line = String::new();
                input
                    .by_ref()
                    .take((MAX_FRAME + 1) as u64)
                    .read_line(&mut line)
                    .unwrap();
                assert!(line.len() <= MAX_FRAME);
                let actual: Value = serde_json::from_str(line.trim()).unwrap();
                assert!(matches_subset(&actual, &frame), "{actual} != {frame}");
            }
            Step::Emit { frame } => {
                println!("{frame}");
                std::io::stdout().flush().unwrap();
            }
        }
    }
    std::fs::write(
        std::env::var_os("KIVIO_REPLAY_RECEIPT").unwrap(),
        "complete",
    )
    .unwrap();
    // libtest's footer is not a protocol frame. Exit only this dedicated peer process.
    std::process::exit(0);
}

#[test]
fn replay_loader_rejects_oversized_or_unattributed_records_and_matches_exact_values() {
    let header = r#"{"type":"transcript_start","protocol":"test","version":"1","classification":"synthetic"}"#;
    assert!(load(&" ".repeat(MAX_TRANSCRIPT + 1)).is_err());
    assert!(load(&format!("{header}\n{}", " ".repeat(MAX_FRAME + 1))).is_err());
    assert!(load("{}\n").is_err());
    assert!(!matches_subset(
        &serde_json::json!({"id":"1"}),
        &serde_json::json!({"id":1})
    ));
    assert!(!matches_subset(&Value::Null, &serde_json::json!({})));
    assert!(matches_subset(
        &serde_json::json!({"id":1,"extra":true}),
        &serde_json::json!({"id":1})
    ));
    for name in [
        "codex-real-turn",
        "pi-real-turn",
        "acp-adapted-grok-turn",
        "claude-recorded-approval",
        "claude-disconnect",
        "dsh-two-turns",
        "dsh-disconnect",
        "dsh-stop-next",
        "antigravity-steps",
    ] {
        assert!(!fixture(name).is_empty());
    }
}
