#!/usr/bin/env python3
"""An ACP peer that records prompts and native session bindings for the desktop probes."""
import json
import sys
import uuid
from pathlib import Path

args = sys.argv[1:]
if args == ["--version"]:
    print("1.5.10")
    sys.exit(0)
if args == ["models"]:
    print("fixture-model")
    sys.exit(0)
assert args and args[0] == "acp", args

root = Path(__file__).parent
session = None


def update(kind, text):
    print(json.dumps({"jsonrpc": "2.0", "method": "session/update", "params": {
        "sessionId": session, "update": {"sessionUpdate": kind,
            "content": {"type": "text", "text": text}},
    }}), flush=True)


for line in sys.stdin:
    request = json.loads(line)
    method = request["method"]
    params = request.get("params", {})
    if method == "initialize":
        result = {"protocolVersion": 1, "agentCapabilities": {
            "loadSession": True, "sessionCapabilities": {"list": {}, "resume": {}}},
            "agentInfo": {"name": "Antigravity ACP Server", "version": "1.5.10"}}
    elif method in {"session/new", "session/resume", "session/load"}:
        requested = params.get("sessionId")
        session = requested or str(uuid.uuid4())
        transcript = root / f"{session}.jsonl"
        exists = transcript.is_file()
        with (root / "starts.jsonl").open("a") as log:
            print(json.dumps({"requested": requested, "native": session if not requested or exists else None}), file=log)
        if method == "session/resume" and not exists:
            print(json.dumps({"jsonrpc": "2.0", "id": request["id"], "error": {"code": -32603,
                "message": f"Failed to read session {session}; retry, or load it with session/load"}}), flush=True)
            continue
        if method == "session/load" and exists:
            for saved in transcript.read_text().splitlines():
                update("user_message_chunk", json.loads(saved))
        # agy 1.5.10 also acknowledges an unknown id, with no history.
        result = {"sessionId": session}
    elif method == "session/list":
        result = {"sessions": [{"sessionId": p.stem, "cwd": params.get("cwd")}
                                for p in root.glob("*.jsonl") if p.name != "starts.jsonl"]}
    elif method in {"session/set_model", "session/set_config_option", "session/set_mode"}:
        assert params["sessionId"] == session
        result = {}
    elif method == "session/prompt":
        assert params["sessionId"] == session
        prompt = "\n".join(block["text"] for block in params["prompt"] if block["type"] == "text")
        with transcript.open("a") as log:
            print(json.dumps(prompt), file=log)
        number = len(transcript.read_text().splitlines())
        update("agent_message_chunk", f"receipt {session}/{number}")
        result = {"stopReason": "end_turn"}
    else:
        raise AssertionError(request)
    print(json.dumps({"jsonrpc": "2.0", "id": request["id"], "result": result}), flush=True)
