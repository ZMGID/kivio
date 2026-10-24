use kivio_model_runtime::{native_provider_probe, settings::ModelProvider};
use serde_json::json;
use std::io::{Read, Write};
#[tokio::test]
async fn draft_provider_probe_uses_selected_key_headers_and_protocol() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    listener.set_nonblocking(true).unwrap();
    let server = std::thread::spawn(move || {
        let mut requests = vec![];
        for body in [
            r#"{"models":[{"name":"models/gemini-test","supportedGenerationMethods":["generateContent"]},{"name":"models/embed-only","supportedGenerationMethods":["embedContent"]}]}"#,
            r#"{"candidates":[]}"#,
        ] {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            let (mut socket, _) = loop {
                match listener.accept() {
                    Ok(value) => break value,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(
                            std::time::Instant::now() < deadline,
                            "probe did not reach server"
                        );
                        std::thread::sleep(std::time::Duration::from_millis(5));
                    }
                    Err(error) => panic!("{error}"),
                }
            };
            socket.set_nonblocking(false).unwrap();
            socket
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            let mut bytes = vec![];
            loop {
                let mut chunk = [0; 4096];
                let count = socket.read(&mut chunk).unwrap();
                assert!(count > 0);
                bytes.extend_from_slice(&chunk[..count]);
                if let Some(end) = bytes.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&bytes[..end]);
                    let length = headers
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if bytes.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            requests.push(String::from_utf8(bytes).unwrap());
            write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        }
        requests
    });
    let provider: ModelProvider = serde_json::from_value(json!({"id":"probe","name":"Draft","baseUrl":base,"apiFormat":"gemini","apiKeys":["first-fixture-key","chosen-fixture-key"],"activeKeyIndex":1,"enabled":true,"enabledModels":["gemini-test"],"request":{"useSystemProxy":false,"customHeaders":[{"key":"x-draft-setting","value":"draft-value"}]}})).unwrap();
    let root = tempfile::tempdir().unwrap();
    assert_eq!(
        native_provider_probe::models(&provider, root.path())
            .await
            .unwrap(),
        vec!["gemini-test"]
    );
    native_provider_probe::connection(&provider, root.path())
        .await
        .unwrap();
    let requests = server.join().unwrap();
    assert!(requests[0].starts_with("GET /v1/models?pageSize=1000 HTTP/1.1"));
    assert!(requests[1].starts_with("POST /v1/models/gemini-test:generateContent HTTP/1.1"));
    for request in &requests {
        assert!(request.contains("x-draft-setting: draft-value"));
    }
    assert!(requests[1].contains("chosen-fixture-key"));
    assert!(!requests[1].contains("first-fixture-key"));
    let body: serde_json::Value =
        serde_json::from_str(requests[1].split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(
        body["contents"][0]["parts"][0]["text"],
        "Reply with exactly: kivio-ok"
    );
}
