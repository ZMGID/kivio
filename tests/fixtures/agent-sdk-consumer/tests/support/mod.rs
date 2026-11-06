use kivio_agent_sdk::model::openai::{OpenAiChatConfig, OpenAiChatProvider};
use serde_json::{json, Value};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::Notify,
    task::JoinHandle,
    time::timeout,
};

pub const LIMIT: Duration = Duration::from_secs(5);

pub enum Response {
    Sse(String),
    Http(u16, String),
    Disconnect(String),
    Hold(String),
}

pub struct Request {
    pub headers: String,
    pub body: Value,
}

pub struct Server {
    pub endpoint: reqwest::Url,
    pub requests: Arc<Mutex<Vec<Request>>>,
    pub disconnected: Arc<Notify>,
    task: JoinHandle<()>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Server {
    pub async fn start(responses: Vec<Response>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!(
            "http://{}/v1/chat/completions",
            listener.local_addr().unwrap()
        )
        .parse()
        .unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = requests.clone();
        let disconnected = Arc::new(Notify::new());
        let closed = disconnected.clone();
        let task = tokio::spawn(async move {
            let mut responses = VecDeque::from(responses);
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let request = timeout(LIMIT, read_request(&mut socket)).await.unwrap();
                recorded.lock().unwrap().push(request);
                let response = responses
                    .pop_front()
                    .unwrap_or_else(|| Response::Http(500, "unexpected extra request".into()));
                let (status, body, extra_length, hold) = match response {
                    Response::Sse(body) => (200, body, 0, false),
                    Response::Http(status, body) => (status, body, 0, false),
                    Response::Disconnect(body) => (200, body, 100, false),
                    Response::Hold(body) => (200, body, 10000, true),
                };
                let headers = format!("HTTP/1.1 {status} Test\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len() + extra_length);
                socket.write_all(headers.as_bytes()).await.unwrap();
                socket.write_all(body.as_bytes()).await.unwrap();
                socket.flush().await.unwrap();
                if hold {
                    let mut byte = [0];
                    let read = timeout(LIMIT, socket.read(&mut byte)).await.unwrap();
                    assert!(
                        matches!(read, Ok(0) | Err(_)),
                        "client did not cancel its stream"
                    );
                    closed.notify_one();
                }
            }
        });
        Self {
            endpoint,
            requests,
            disconnected,
            task,
        }
    }

    pub fn provider(&self) -> OpenAiChatProvider {
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(LIMIT)
            .build()
            .unwrap();
        let mut config = OpenAiChatConfig::new(self.endpoint.clone());
        config.api_key = Some("fixture-key".into());
        config
            .headers
            .insert("x-consumer", "standalone".parse().unwrap());
        OpenAiChatProvider::new(client, config)
    }

    pub fn count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
}

async fn read_request(socket: &mut TcpStream) -> Request {
    let mut bytes = Vec::new();
    let header_end = loop {
        let mut chunk = [0; 2048];
        let length = socket.read(&mut chunk).await.unwrap();
        assert!(length > 0, "request closed before headers");
        bytes.extend_from_slice(&chunk[..length]);
        assert!(bytes.len() < 1024 * 1024, "request exceeded fixture bound");
        if let Some(index) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            break index + 4;
        }
    };
    let headers = String::from_utf8(bytes[..header_end].to_vec()).unwrap();
    assert!(headers.starts_with("POST /v1/chat/completions HTTP/1.1"));
    let length: usize = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse().unwrap())
        })
        .expect("real request must declare its body length");
    assert!(length < 1024 * 1024);
    while bytes.len() < header_end + length {
        let mut chunk = [0; 2048];
        let received = socket.read(&mut chunk).await.unwrap();
        assert!(received > 0, "request closed before body");
        bytes.extend_from_slice(&chunk[..received]);
    }
    Request {
        headers,
        body: serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap(),
    }
}

pub fn event(value: Value) -> String {
    format!("data: {value}\n\n")
}

pub fn text_delta(text: &str) -> String {
    event(json!({"choices":[{"index":0,"delta":{"content":text}}]}))
}

pub fn answer(text: &str) -> Response {
    Response::Sse(format!(
        "{}{}data: [DONE]\n\n",
        text_delta(text),
        event(json!({"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}))
    ))
}

pub fn tool(name: &str, arguments: &str) -> Response {
    Response::Sse(format!(
        "{}{}data: [DONE]\n\n",
        event(
            json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call-42","type":"function","function":{"name":name,"arguments":arguments}}]}}]})
        ),
        event(json!({"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}))
    ))
}
