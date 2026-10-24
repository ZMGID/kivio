#!/usr/bin/env python3
"""ACP wire peer: restore, permission roundtrip, streamed tools and turn completion."""
import json
import os
import sys


def emit(value):
    print(json.dumps(value), flush=True)


def update(value):
    emit({"jsonrpc": "2.0", "method": "session/update", "params": {
        "sessionId": "acp-session", "update": value}})


pending = None
for line in sys.stdin:
    request = json.loads(line)
    with open(os.environ["KIVIO_CLI_TEST_LOG"], "a") as log:
        log.write(json.dumps({"pid": os.getpid(), "request": request}) + "\n")
    method = request.get("method")
    rid = request.get("id")
    result = {}
    if method == "initialize":
        result = {"protocolVersion": 1, "agentCapabilities": {"loadSession": True}}
    elif method == "session/new":
        result = {"sessionId": "acp-session"}
    elif method == "session/load":
        if os.path.exists(os.environ["KIVIO_CLI_TEST_LOG"] + ".reject-resume"):
            emit({"jsonrpc": "2.0", "id": rid, "error": {"code": -32000, "message": "session missing"}})
            continue
        assert request["params"]["sessionId"] == "acp-session"
    elif method == "session/prompt":
        pending = rid
        update({"sessionUpdate": "agent_message_chunk", "content": {
            "type": "text", "text": "ACP reply"}})
        emit({"jsonrpc": "2.0", "id": "permission", "method": "session/request_permission",
              "params": {"sessionId": "acp-session", "toolCall": {"title": "Run command?"},
                         "options": [{"optionId": "deny", "name": "拒绝", "kind": "reject_once"},
                                     {"optionId": "allow", "name": "允许本次", "kind": "allow_once"}]}})
        continue
    elif rid == "permission":
        assert request["result"]["outcome"] == {"outcome": "selected", "optionId": "deny"}
        emit({"jsonrpc": "2.0", "id": pending, "result": {"stopReason": "end_turn"}})
        continue
    elif method == "session/cancel":
        break
    elif method not in ("session/set_model", "session/set_config_option", "initialized"):
        raise AssertionError(request)
    if rid is not None:
        emit({"jsonrpc": "2.0", "id": rid, "result": result})
