#!/usr/bin/env python3
"""Claude stream-json peer with an explicit tool permission roundtrip."""
import json
import os
import sys


def emit(value):
    print(json.dumps(value), flush=True)


for line in sys.stdin:
    request = json.loads(line)
    with open(os.environ["KIVIO_CLI_TEST_LOG"], "a") as log:
        log.write(json.dumps({"pid": os.getpid(), "args": sys.argv[1:], "request": request}) + "\n")
    if request["type"] == "control_request":
        emit({"type": "control_response", "response": {
            "subtype": "success", "request_id": request["request_id"], "response": {}}})
    elif request["type"] == "user":
        emit({"type": "system", "subtype": "init", "session_id": "claude-session", "cwd": os.getcwd()})
        emit({"type": "stream_event", "event": {"type": "content_block_delta",
            "delta": {"type": "text_delta", "text": "Claude reply"}}})
        emit({"type": "control_request", "request_id": "permission", "request": {
            "subtype": "can_use_tool", "tool_name": "Bash", "input": {"command": "echo hello"}}})
    elif request["type"] == "control_response":
        assert request["response"]["response"]["behavior"] == "deny"
        emit({"type": "result", "subtype": "success", "session_id": "claude-session",
              "result": "Claude reply", "usage": {"input_tokens": 10, "output_tokens": 2}})
