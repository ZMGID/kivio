"""Verify Antigravity recovery across macOS Test app restarts using a temporary profile."""
import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
import uuid
from pathlib import Path

IDENTIFIER = "com.zmair.kivio.test"
PEER = Path(__file__).resolve().parents[1] / "tests/fixtures/external-cli/agy.py"


def read_json(path):
    return json.loads(path.read_text())


def read_lines(path):
    return [json.loads(line) for line in path.read_text().splitlines()]


def run(binary, home):
    data = home / "Library/Application Support" / IDENTIFIER
    probe = data / "chat_probe"
    workspace = home / "workspace"
    peer = home / "peer/agy"
    for directory in (data, workspace, peer.parent):
        directory.mkdir(parents=True)
    shutil.copyfile(PEER, peer)
    peer.chmod(0o700)
    settings = {
        "autoCheckUpdate": False, "launchAtStartup": False, "onboardingStatus": "completed",
        "chatMemory": {"enabled": False},
        "chat": {"externalCliAgents": {"antigravity": {"path": str(peer)}}},
    }
    (data / "settings.json").write_text(json.dumps({"settings": settings}))
    env = {
        "HOME": str(home), "CFFIXED_USER_HOME": str(home), "SHELL": "/bin/sh",
        "PATH": os.pathsep.join([str(peer.parent), str(Path(sys.executable).parent), "/usr/bin", "/bin"]),
        "LANG": "en_US.UTF-8", "HTTP_PROXY": "http://127.0.0.1:1", "HTTPS_PROXY": "http://127.0.0.1:1",
        "NO_PROXY": "localhost,127.0.0.1",
    }
    chat = None
    messages = []

    def turn(label, prompt):
        nonlocal chat
        logfile = home / f"{label}.log"
        with logfile.open("wb") as log:
            app = subprocess.Popen([binary, "--from-autostart"], cwd=workspace,
                                   stdin=subprocess.DEVNULL, stdout=log, stderr=log, env=env)
        try:
            def wait(predicate, description):
                deadline = time.monotonic() + 60
                while app.poll() is None and time.monotonic() < deadline:
                    value = predicate()
                    if value:
                        return value
                    time.sleep(0.1)
                tail = logfile.read_text(errors="replace")[-3000:]
                raise AssertionError(f"{label}: {description}, Kivio exit={app.poll()}\n{tail}")

            wait(lambda: f"[chat-probe] watching {probe}/request.json" in logfile.read_text(errors="replace"),
                 "isolated Test probe never started (close any other Kivio Test instance)")
            request = {"id": label, "prompt": prompt, "externalAgentId": "antigravity", "externalModel": "fixture-model",
                       "cwd": str(workspace), "webSearchMode": "off", "timeoutSeconds": 30}
            if chat:
                request["conversationId"] = chat
            staged = probe / f"{label}.tmp"
            staged.write_text(json.dumps(request))
            staged.rename(probe / "request.json")
            result_file = probe / f"result-{label}.json"

            def completed_result():
                # The watcher writes this file directly, so existence can precede valid JSON.
                try:
                    return read_json(result_file)
                except (FileNotFoundError, json.JSONDecodeError):
                    return None

            result = wait(completed_result, "missing or incomplete turn result")
            assert result.get("streamOutcome") == "completed" and not result.get("error"), result
            assert chat is None or result["conversationId"] == chat, result
            chat = result["conversationId"]
            bindings = data / "external-agent-sessions"
            handle = bindings / f"live-{chat}.json"
            native = read_json(handle)["native_id"]
            assert read_json(bindings / f"{chat}.json")["sessionId"] == native
            conversation = read_json(data / "conversations" / f"{chat}.json")
            assert conversation.get("external_history") is None, "Successful delivery was not settled"
            receipt = result["answer"]
            messages.extend([("user", prompt), ("assistant", receipt)])
            actual = [(m["role"], m["content"]) for m in conversation["messages"]]
            assert actual == messages, (label, actual)
            return native, receipt, handle
        finally:
            app.terminate()
            try:
                app.wait(timeout=5)
            except subprocess.TimeoutExpired:
                app.kill()
                app.wait()

    seed, recover, followup = "Remember juniper.", "Recall my tree.", "Continue after reopening."
    original, original_answer, handle = turn("seed", seed)
    assert original_answer == f"receipt {original}/1"
    original_file = peer.parent / f"{original}.jsonl"
    original_bytes = original_file.read_bytes()
    [first_prompt] = read_lines(original_file)
    assert first_prompt.count(seed) == 1

    # The live resume reference is stale. Keep the old native transcript and ordinary binding intact.
    missing = str(uuid.uuid4())
    binding = read_json(handle)
    binding["protocol"] = "antigravity_stream_json"
    binding["native_id"] = missing
    handle.write_text(json.dumps(binding))

    replacement, answer, _ = turn("recover", recover)
    assert replacement not in (original, missing)
    assert answer == f"receipt {replacement}/1"
    starts = peer.parent / "starts.jsonl"
    connections = read_lines(starts)
    assert {"requested": missing, "native": None} in connections, "Kivio did not attempt the previous protocol's binding"

    assert turn("reopen", followup)[:2] == (replacement, f"receipt {replacement}/2")
    reopened = read_lines(starts)[len(connections):]
    assert any(s["requested"] == replacement for s in reopened), reopened
    recovery_prompt, next_prompt = read_lines(peer.parent / f"{replacement}.jsonl")
    for text in (seed, original_answer, recover):
        assert recovery_prompt.count(text) == 1, f"Recovery prompt must contain {text!r} once: {recovery_prompt!r}"
    assert recovery_prompt.index(seed) < recovery_prompt.index(original_answer) < recovery_prompt.index(recover)
    for old in (seed, original_answer, recover, answer):
        assert old not in next_prompt, f"Reopening replayed {old!r}"
    assert next_prompt.count(followup) == 1
    assert original_file.read_bytes() == original_bytes
    assert not (peer.parent / f"{missing}.jsonl").exists()
    print("PASS Antigravity recovered visible history once and resumed the persisted replacement after restart")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    binary = parser.parse_args().binary.resolve()
    if sys.platform != "darwin" or not __debug__:
        parser.error("Run on macOS with Python assertions enabled")
    identity = subprocess.check_output([binary, "--print-app-identifier"], text=True, timeout=10).strip()
    if identity != IDENTIFIER:
        parser.error(f"Expected a Kivio Test executable, got {identity!r}")
    with tempfile.TemporaryDirectory(prefix="kivio-agy-recovery-") as temporary:
        run(binary, Path(temporary).resolve())
