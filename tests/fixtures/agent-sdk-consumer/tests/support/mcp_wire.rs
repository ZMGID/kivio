use serde_json::{json, Value};

#[derive(Default)]
pub struct WireState {
    pub calls: Vec<Value>,
    pub cancelled: Vec<Value>,
    pub lists: usize,
    pub changed: bool,
    notify_on_page: bool,
}

impl WireState {
    pub fn receive(&mut self, request: Value) -> Vec<Value> {
        let id = request["id"].clone();
        let method = request["method"].as_str().unwrap_or_default();
        let params = &request["params"];
        if method == "notifications/cancelled" {
            self.cancelled.push(params["requestId"].clone());
            return vec![];
        }
        if id.is_null() {
            return vec![];
        }
        let mut notifications = vec![];
        let result = match method {
            "initialize" => json!({
                "protocolVersion":"2025-06-18", "capabilities":{"tools":{"listChanged":true}},
                "serverInfo":{"name":"consumer-fixture","version":"1"}
            }),
            "ping" => json!({}),
            "tools/list" => {
                self.lists += 1;
                if params["cursor"].is_null() {
                    json!({"tools":[{
                        "name":"echo", "description":"Echo with image and structured data",
                        "inputSchema":{"type":"object","properties":{"text":{"type":"string"}},"required":["text"]},
                        "annotations":{"readOnlyHint":true}
                    }], "nextCursor":"page-2"})
                } else {
                    assert_eq!(params["cursor"], "page-2");
                    if std::mem::take(&mut self.notify_on_page) {
                        notifications.push(
                            json!({"jsonrpc":"2.0","method":"notifications/tools/list_changed"}),
                        );
                    }
                    let mut tools: Vec<Value> = ["wait", "stats", "refresh", "lost"]
                        .into_iter()
                        .map(|name| json!({"name":name,"inputSchema":{"type":"object"}}))
                        .collect();
                    if self.changed {
                        tools.push(json!({"name":"added","inputSchema":{"type":"object"}}));
                    }
                    json!({"tools":tools})
                }
            }
            "tools/call" => {
                self.calls.push(params.clone());
                match params["name"].as_str().unwrap_or_default() {
                    "wait" | "lost" => return vec![],
                    "refresh" => {
                        self.changed = true;
                        self.notify_on_page = params["arguments"]["during_page"] == true;
                        notifications.push(
                            json!({"jsonrpc":"2.0","method":"notifications/tools/list_changed"}),
                        );
                        json!({"content":[{"type":"text","text":"refreshed"}]})
                    }
                    "stats" => {
                        json!({"content":[{"type":"text","text":"stats"}],"structuredContent":{
                            "calls":self.calls,"cancelled":self.cancelled,"lists":self.lists
                        }})
                    }
                    _ => json!({"content":[
                        {"type":"text","text":params["arguments"]["text"].as_str().unwrap_or("added")},
                        {"type":"image","data":"aGVsbG8=","mimeType":"image/png"}
                    ],"structuredContent":{"echo":params["arguments"]["text"]},
                       "isError":params["arguments"]["text"] == "error"}),
                }
            }
            _ => {
                return vec![
                    json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"Method not found"}}),
                ]
            }
        };
        notifications.push(json!({"jsonrpc":"2.0","id":id,"result":result}));
        notifications
    }
}
