use serde_json::{json, Value};
use std::{collections::VecDeque, io, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::JoinHandle,
    time::timeout,
};

pub struct Server {
    pub endpoint: reqwest::Url,
    task: JoinHandle<io::Result<()>>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Server {
    pub async fn start(inputs: bool, continuation: bool) -> io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let endpoint = format!("http://{}/v1/chat/completions", listener.local_addr()?)
            .parse()
            .expect("loopback URL");
        let mut responses = VecDeque::from([
            format!(
                "{}{}data: [DONE]\n\n",
                event(json!({"choices":[{"index":0,"delta":{"tool_calls":[{
                    "index":0,"id":"echo-1","type":"function","function":{
                        "name":"echo","arguments":"{\"text\":\"hello\"}"}
                }]}}]})),
                event(json!({"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}))
            ),
            answer("Echo returned hello."),
        ]);
        if inputs || continuation {
            responses.push_back(answer("Continued with the supplied history."));
        }
        let task = tokio::spawn(async move {
            while let Some(body) = responses.pop_front() {
                let (mut socket, _) = listener.accept().await?;
                timeout(Duration::from_secs(5), read_request(&mut socket)).await??;
                let headers = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                socket.write_all(headers.as_bytes()).await?;
                socket.write_all(body.as_bytes()).await?;
                socket.shutdown().await?;
            }
            Ok(())
        });
        Ok(Self { endpoint, task })
    }

    pub async fn finish(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        timeout(Duration::from_secs(5), &mut self.task).await???;
        Ok(())
    }
}

async fn read_request(socket: &mut TcpStream) -> io::Result<()> {
    let mut bytes = Vec::new();
    let header_end = loop {
        let mut chunk = [0; 2048];
        let length = socket.read(&mut chunk).await?;
        if length == 0 || bytes.len() + length > 1024 * 1024 {
            return Err(io::Error::other("incomplete or oversized example request"));
        }
        bytes.extend_from_slice(&chunk[..length]);
        if let Some(index) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            break index + 4;
        }
    };
    let headers = String::from_utf8_lossy(&bytes[..header_end]);
    let length: usize = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .filter(|length| *length <= 1024 * 1024)
        .ok_or_else(|| io::Error::other("missing or oversized Content-Length"))?;
    while bytes.len() < header_end + length {
        let mut chunk = [0; 2048];
        let received = socket.read(&mut chunk).await?;
        if received == 0 {
            return Err(io::Error::other("incomplete example request body"));
        }
        bytes.extend_from_slice(&chunk[..received]);
    }
    let body: Value = serde_json::from_slice(&bytes[header_end..header_end + length])?;
    if body["model"] != "example-model" || body["stream"] != true {
        return Err(io::Error::other(
            "expected a streamed example-model request",
        ));
    }
    Ok(())
}

fn event(value: Value) -> String {
    format!("data: {value}\n\n")
}

fn answer(text: &str) -> String {
    format!(
        "{}{}data: [DONE]\n\n",
        event(json!({"choices":[{"index":0,"delta":{"content":text}}]})),
        event(json!({"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}))
    )
}
