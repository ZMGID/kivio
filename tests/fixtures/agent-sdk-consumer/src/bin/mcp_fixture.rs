#[path = "../../tests/support/mcp_wire.rs"]
mod mcp_wire;

use std::io::{BufRead, Write};

fn main() {
    let mut state = mcp_wire::WireState::default();
    let mut output = std::io::stdout().lock();
    for line in std::io::stdin().lock().lines() {
        let request: serde_json::Value = serde_json::from_str(&line.unwrap()).unwrap();
        let lost = request["method"] == "tools/call" && request["params"]["name"] == "lost";
        for reply in state.receive(request) {
            writeln!(output, "{reply}").unwrap();
            output.flush().unwrap();
        }
        if lost {
            break;
        }
    }
}
