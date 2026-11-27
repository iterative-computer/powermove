#!/usr/bin/python3
"""A synthetic Codex executable for Project and Full access launch tests."""
import json
import os
from pathlib import Path
import sys
import time

scenario = os.environ["PM_TEST_SCENARIO"]

def emit(value):
    print(json.dumps(value), flush=True)

def respond(request, value):
    emit({"id": request["id"], "result": value})

def notify(method, **params):
    emit({"method": method, "params": {"threadId": "thread", **params}})

def wait_for(name):
    deadline = time.monotonic() + 5
    while not Path(name).exists():
        if time.monotonic() >= deadline:
            raise SystemExit("test barrier timed out: " + name)
        time.sleep(0.005)

if sys.argv[1] != "app-server":
    Path("exec-called").touch()
    assert scenario == "startup-failure"
    args = sys.argv[1:]
    assert "exec" in args
    prompt = args[-1]
    resumed = "resume" in args
    if resumed:
        assert args[-2] == "exec-session"
        assert "Make it blue\n\nKeep it short" in prompt
        reply = "Short blue answer"
    else:
        assert "Keep the original brief" in prompt
        assert "Make it blue" not in prompt
        reply = "Original answer"
    emit({"type": "thread.started", "thread_id": "exec-session"})
    output = Path(args[args.index("--output-last-message") + 1])
    output.write_text(json.dumps({"reply": reply}), encoding="utf-8")
    emit({"type": "turn.completed"})
    raise SystemExit(0)

steers = []
for line in sys.stdin:
    request = json.loads(line)
    method = request.get("method")
    params = request.get("params", {})
    if method == "initialize":
        if scenario == "startup-failure":
            wait_for("startup-release")
            emit({"id": request["id"], "error": {"code": -32603, "message": "Synthetic startup failure"}})
        else:
            respond(request, {})
    elif method == "thread/start":
        if scenario == "full-steer":
            assert params["approvalPolicy"] == "never"
            assert params["sandbox"] == "danger-full-access"
        else:
            assert params["approvalPolicy"] == "on-request"
            assert params["approvalsReviewer"] == "user"
            assert params["config"]["default_permissions"] == "powermove"
            assert params["permissions"] == "powermove" and "sandbox" not in params
        respond(request, {"thread": {"id": "thread"}})
    elif method == "turn/start":
        assert "Keep the original brief" in params["input"][0]["text"]
        respond(request, {"turn": {"id": "turn"}})
        notify("item/started", turnId="turn", item={
            "type": "commandExecution", "id": "work", "command": "synthetic"
        })
        if scenario == "crash":
            raise SystemExit(17)
    elif method == "turn/steer":
        assert params["threadId"] == "thread" and params["expectedTurnId"] == "turn"
        steers.append(params["input"][0]["text"])
        respond(request, {"turnId": "turn"})
        if len(steers) == 2:
            assert steers == ["Make it blue", "Keep it short"]
            if scenario == "full-steer":
                notify("item/completed", turnId="turn", item={
                    "type": "agentMessage", "id": "answer", "text": '{"reply":"Short blue answer"}'
                })
                notify("turn/completed", turn={"id": "turn", "status": "completed"})
            else:
                emit({"id": "permission-1", "method": "item/commandExecution/requestApproval",
                      "params": {"threadId": "thread", "turnId": "turn", "itemId": "work",
                                 "command": "synthetic outside project", "reason": "Test approval"}})
    elif request.get("id") == "permission-1":
        assert request["result"]["decision"] == "accept"
        notify("item/completed", turnId="turn", item={
            "type": "agentMessage", "id": "answer", "text": '{"reply":"Short blue answer"}'
        })
        notify("turn/completed", turn={"id": "turn", "status": "completed"})
