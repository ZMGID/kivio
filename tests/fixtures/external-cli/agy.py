#!/usr/bin/env python3
"""A local stream-json peer. Store only what the CLI actually receives on stdin."""
import json
import sys
import uuid
from pathlib import Path

args = sys.argv[1:]
if args == ["--version"]:
    print("agy 1.2.6")
    sys.exit(0)
if args == ["models"]:
    print("fixture-model")
    sys.exit(0)
if "--input-format" not in args or args[args.index("--input-format") + 1] != "stream-json":
    sys.exit(f"Unexpected Antigravity invocation: {args}")

root = Path(__file__).parent
requested = args[args.index("--conversation") + 1] if "--conversation" in args else None
native = requested
if not native or not (root / f"{native}.jsonl").is_file():
    native = str(uuid.uuid4())
    if requested:
        print(f'warning: conversation "{requested}" not found', file=sys.stderr, flush=True)

with (root / "starts.jsonl").open("a") as log:
    print(json.dumps({"requested": requested, "native": native}), file=log)
print(json.dumps({"event": "init", "conversation_id": native}), flush=True)
for line in sys.stdin:
    frame = json.loads(line)
    assert frame["event"] == "user", frame
    transcript = root / f"{native}.jsonl"
    with transcript.open("a") as log:
        print(json.dumps(frame["message"]["content"]), file=log)
    number = len(transcript.read_text().splitlines())
    print(json.dumps({"event": "result", "result": {
        "conversation_id": native, "status": "SUCCESS", "response": f"receipt {native}/{number}"
    }}), flush=True)
