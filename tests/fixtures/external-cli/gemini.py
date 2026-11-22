#!/usr/bin/env python3
"""A local ACP peer that records the prompts it receives from Kivio."""
import json
import sys
import uuid
from pathlib import Path

if sys.argv[1:] == ["--version"]:
    print("0.18.0")
    sys.exit(0)
assert sys.argv[1:] == ["--acp"], sys.argv

root = Path(__file__).parent
for line in sys.stdin:
    request = json.loads(line)
    method = request["method"]
    params = request.get("params", {})
    if method == "initialize":
        result = {"protocolVersion": 1}
    elif method == "session/new":
        session = str(uuid.uuid4())
        result = {"sessionId": session}
    elif method in {"session/set_model", "session/set_config_option", "session/set_mode"}:
        assert params["sessionId"] == session, request
        result = {}
    elif method == "session/prompt":
        assert params["sessionId"] == session, request
        transcript = root / f"{session}.jsonl"
        with transcript.open("a") as log:
            print(json.dumps(params["prompt"]), file=log)
        receipt = f"Gemini receipt {session}/{len(transcript.read_text().splitlines())}"
        print(json.dumps({"jsonrpc": "2.0", "method": "session/update", "params": {
            "sessionId": session, "update": {"sessionUpdate": "agent_message_chunk",
                "content": {"type": "text", "text": receipt}},
        }}), flush=True)
        result = {"stopReason": "end_turn"}
    else:
        raise AssertionError(request)
    print(json.dumps({"jsonrpc": "2.0", "id": request["id"], "result": result}), flush=True)
