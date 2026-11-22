"""Exercise runtime switching and sending over the real macOS Test app IPC."""
import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

IDENTIFIER = "com.zmair.kivio.test"
FIXTURES = Path(__file__).resolve().parents[1] / "tests/fixtures/external-cli"
PROVIDER = "runtime-switch-fixture"
MODEL = "fixture-model"


def read_json(path):
    return json.loads(path.read_text())


def read_lines(path):
    return [json.loads(line) for line in path.read_text().splitlines()]


def text_content(value):
    if isinstance(value, str):
        return value
    return "\n".join(part["text"] for part in value if part["type"] == "text")


def model_server(trace):
    class Handler(BaseHTTPRequestHandler):
        def do_POST(self):
            assert self.path == "/v1/chat/completions", self.path
            body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            assert body["model"] == MODEL, body
            with trace.open("a") as log:
                print(json.dumps(body["messages"]), file=log)
            choice = {"index": 0, "finish_reason": "stop"}
            message = {"role": "assistant", "content": f"model receipt {uuid.uuid4()}"}
            self.send_response(200)
            if body.get("stream"):
                self.send_header("Content-Type", "text/event-stream")
                self.end_headers()
                for event in ({"choices": [{"index": 0, "delta": message}]},
                              {"choices": [{**choice, "delta": {}}]}):
                    self.wfile.write(f"data: {json.dumps(event)}\n\n".encode())
                self.wfile.write(b"data: [DONE]\n\n")
            else:
                self.send_header("Content-Type", "application/json")
                self.end_headers()
                self.wfile.write(json.dumps({"id": "fixture", "model": MODEL,
                    "object": "chat.completion", "created": 0,
                    "choices": [{**choice, "message": message}]}).encode())

        def log_message(self, *_):
            pass

    return ThreadingHTTPServer(("127.0.0.1", 0), Handler)


def run(binary, home, base, trace):
    data = home / "Library/Application Support" / IDENTIFIER
    probe = data / "chat_probe"
    workspace = home / "workspace"
    antigravity, gemini = home / "agy/agy", home / "gemini/gemini"
    for directory in (data, workspace, antigravity.parent, gemini.parent):
        directory.mkdir(parents=True)
    for peer in (antigravity, gemini):
        shutil.copyfile(FIXTURES / f"{peer.name}.py", peer)
        peer.chmod(0o700)

    def cli(agent):
        return {"kind": "external", "externalAgentId": agent, "externalModel": MODEL}

    builtin = {"kind": "builtin"}
    settings = {"autoCheckUpdate": False, "launchAtStartup": False, "onboardingStatus": "completed",
        "hotkey": "", "chatHotkey": "", "closeChatHotkey": "", "chatMemory": {"enabled": False},
        "providers": [{"id": PROVIDER, "name": "Switch fixture", "enabled": True,
            "apiFormat": "openai_chat", "apiKeys": ["fixture-only"], "baseUrl": base,
            "availableModels": [MODEL], "enabledModels": [MODEL]}],
        "defaultModels": {"chat": {"providerId": PROVIDER, "model": MODEL}},
        "chatProviderId": PROVIDER, "chatModel": MODEL,
        "chatTools": {"enabled": False, "skillAutoMatch": False},
        "chat": {"thinkingEnabled": False, "streamEnabled": True,
            "externalCliAgents": {"antigravity": {"path": str(antigravity)}, "gemini": {"path": str(gemini)}}},
    }
    (data / "settings.json").write_text(json.dumps({"settings": settings}))
    env = {"HOME": str(home), "CFFIXED_USER_HOME": str(home), "SHELL": "/bin/sh",
        "PATH": os.pathsep.join([str(antigravity.parent), str(gemini.parent),
            str(Path(sys.executable).parent), "/usr/bin", "/bin"]),
        "LANG": "en_US.UTF-8", "HTTP_PROXY": "http://127.0.0.1:1", "HTTPS_PROXY": "http://127.0.0.1:1",
        "NO_PROXY": "localhost,127.0.0.1"}
    logfile = home / "app.log"
    with logfile.open("wb") as log:
        app = subprocess.Popen([binary], cwd=workspace, env=env,
            stdin=subprocess.DEVNULL, stdout=log, stderr=log)

    def wait(read, description):
        deadline = time.monotonic() + 45
        while app.poll() is None and time.monotonic() < deadline:
            try:
                value = read()
                if value:
                    return value
            except (FileNotFoundError, json.JSONDecodeError):
                pass
            time.sleep(0.1)
        raise AssertionError(f"{description}, Kivio exit={app.poll()}\n{logfile.read_text(errors='replace')[-3500:]}")

    conversation_id = None
    messages = []
    handoff_anchors = []

    def turn(label, expected, *, switch=False):
        nonlocal conversation_id
        prompt = f"Reply briefly. This is {label}-{uuid.uuid4()}."
        request = {"id": label, "prompt": prompt, "provider": PROVIDER, "model": MODEL,
            "cwd": str(workspace), "sendThroughIpc": True, "webSearchMode": "off", "timeoutSeconds": 30}
        if conversation_id is None:
            request.update(externalAgentId=expected["externalAgentId"], externalModel=MODEL)
        else:
            request["conversationId"] = conversation_id
        if switch:
            request["switchAgentRuntime"] = expected
            handoff_anchors.append(messages[-1][0])
        temporary = probe / f"{label}.tmp"
        temporary.write_text(json.dumps(request))
        temporary.rename(probe / "request.json")
        result = wait(lambda: read_json(probe / f"result-{label}.json"), f"Waiting for {label} result")
        assert result.get("streamOutcome") == "completed" and not result.get("error"), (label, result)
        assert conversation_id is None or result["conversationId"] == conversation_id, result
        conversation_id = result["conversationId"]
        saved = read_json(data / "conversations" / f"{conversation_id}.json")
        assert all(saved["agent_runtime"][key] == value for key, value in expected.items()), saved["agent_runtime"]
        assert [item["after_message_id"] for item in saved.get("agent_handoffs", [])] == handoff_anchors
        assert saved.get("external_history") is None, "Completed send left a pending history handoff"
        actual = [(item["id"], item["role"], item["content"]) for item in saved["messages"]]
        assert actual[:-2] == messages, "A previously saved message changed or was removed"
        assert [item[1:] for item in actual[-2:]] == [("user", prompt), ("assistant", result["answer"])], actual
        messages[:] = actual
        native_id = None
        if expected["kind"] == "external":
            binding = read_json(data / "external-agent-sessions" / f"live-{conversation_id}.json")
            assert binding["agent_id"] == expected["externalAgentId"], binding
            native_id = binding["native_id"]
        return prompt, result["answer"], native_id

    def once_in_order(text, *expected):
        positions = []
        for item in expected:
            assert text.count(item) == 1, f"Expected once in protocol payload: {item!r}\n{text}"
            positions.append(text.index(item))
        assert positions == sorted(positions), text

    def visible_text(start=0):
        return tuple(content for _, _, content in messages[start:])

    try:
        wait(lambda: f"[chat-probe] watching {probe}/request.json" in logfile.read_text(errors="replace"),
            "Test probe did not start (another Kivio Test instance can prevent launch)")
        # The app starts its chat WebView shortly after starting the watcher.
        time.sleep(3)

        seed, _, agy_id = turn("seed-antigravity", cli("antigravity"))
        seed_next, _, initial_followup = turn("continue-original-antigravity", cli("antigravity"))
        known_by_agy = visible_text()
        agy_trace = antigravity.parent / f"{agy_id}.jsonl"
        initial_first, initial_next = read_lines(agy_trace)
        assert initial_followup == agy_id and seed not in initial_next
        once_in_order(initial_first, seed)
        once_in_order(initial_next, seed_next)
        agy_seed = agy_trace.read_bytes()

        to_gemini, _, gemini_id = turn("switch-to-gemini", cli("gemini"), switch=True)
        another_gemini, _, gemini_followup_id = turn("continue-gemini", cli("gemini"))
        assert gemini_followup_id == gemini_id and gemini_id != agy_id
        gemini_first, gemini_next = map(text_content, read_lines(gemini.parent / f"{gemini_id}.jsonl"))
        once_in_order(gemini_first, *known_by_agy, to_gemini)
        assert seed not in gemini_next and to_gemini not in gemini_next
        once_in_order(gemini_next, another_gemini)
        print("PASS Antigravity → Gemini, earlier messages sent once and native session reused", flush=True)

        before_builtin = visible_text()
        to_builtin, builtin_answer, _ = turn("switch-to-builtin", builtin, switch=True)
        another_builtin, _, _ = turn("continue-builtin", builtin)
        # Title generation also uses the configured provider.
        chat_requests = [request for request in read_lines(trace) if any(
            item["role"] == "user" and text_content(item["content"]) in (to_builtin, another_builtin)
            for item in request)]
        model_first, model_next = ["\n".join(text_content(item["content"]) for item in request)
            for request in chat_requests]
        once_in_order(model_first, *before_builtin, to_builtin)
        once_in_order(model_next, *before_builtin, to_builtin, builtin_answer, another_builtin)
        print("PASS Gemini → Kivio Agent, earlier messages included once per model request", flush=True)

        missed_by_agy = visible_text(len(known_by_agy))
        back_to_agy, _, returned_agy_id = turn("switch-back-to-antigravity", cli("antigravity"), switch=True)
        after_return, _, agy_followup_id = turn("continue-returned-antigravity", cli("antigravity"))
        assert returned_agy_id == agy_id == agy_followup_id
        starts = read_lines(antigravity.parent / "starts.jsonl")
        assert {"requested": agy_id, "native": agy_id} in starts, starts
        assert agy_trace.read_bytes().startswith(agy_seed)
        _, _, agy_return, agy_next = read_lines(agy_trace)
        once_in_order(agy_return, *missed_by_agy, back_to_agy)
        assert all(previous not in agy_return for previous in known_by_agy)
        assert seed not in agy_next and to_gemini not in agy_next and back_to_agy not in agy_next
        once_in_order(agy_next, after_return)
        print("PASS Kivio Agent → Antigravity, resumed original native session with only missed turns", flush=True)

        before_gemini_return = visible_text()
        back_to_gemini, _, new_gemini_id = turn("switch-back-to-gemini", cli("gemini"), switch=True)
        after_gemini_return, _, new_gemini_followup = turn("continue-returned-gemini", cli("gemini"))
        assert new_gemini_id != gemini_id and new_gemini_followup == new_gemini_id
        gemini_return, gemini_return_next = map(text_content, read_lines(gemini.parent / f"{new_gemini_id}.jsonl"))
        once_in_order(gemini_return, *before_gemini_return, back_to_gemini)
        assert seed not in gemini_return_next and back_to_gemini not in gemini_return_next
        once_in_order(gemini_return_next, after_gemini_return)
        print(f"PASS Antigravity → Gemini, fresh native session received full portable history, same timeline {conversation_id}")
    finally:
        if app.poll() is None:
            app.terminate()
            try:
                app.wait(timeout=5)
            except subprocess.TimeoutExpired:
                app.kill()
                app.wait()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--keep", action="store_true", help="Keep the isolated profile and raw protocol traces")
    args = parser.parse_args()
    binary = args.binary.resolve()
    if sys.platform != "darwin" or not __debug__:
        parser.error("Run on macOS with Python assertions enabled")
    identity = subprocess.check_output([binary, "--print-app-identifier"], text=True, timeout=10).strip()
    if identity != IDENTIFIER:
        parser.error(f"Expected a Kivio Test executable, got {identity!r}")
    home = Path(tempfile.mkdtemp(prefix="kivio-agent-switch-")).resolve()
    trace = home / "model.jsonl"
    with model_server(trace) as server:
        threading.Thread(target=server.serve_forever, daemon=True).start()
        try:
            run(binary, home, f"http://127.0.0.1:{server.server_port}/v1", trace)
        finally:
            server.shutdown()
            if args.keep:
                print(f"Fixture artifacts: {home}", flush=True)
            else:
                shutil.rmtree(home)
