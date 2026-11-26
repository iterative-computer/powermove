#!/usr/bin/env python3
"""Local stand-ins for Settings › Providers, on 127.0.0.1 only (never the
network): the ChatGPT OAuth issuer Codex signs in with (/auth), the Codex
backend (/chatgpt/backend-api/codex/responses), OpenAI (/openai/v1), Anthropic
(/anthropic/v1), an OpenAI-compatible custom endpoint (/custom/v1) and the
Powermove account's billing and Aero inference (/aero/v2/billing with checkout and
portal, /aero/v2/aero: models, chat completions; the session must be AERO_SESSION).

Usage: fake_backends.py PORT [ACCESS_TTL_SECONDS]. GET /stats answers what it
saw. The model is a fake: asked with a Powermove project in its prompt, it
renames the first layer through the `edit` tool, then says so.
Refresh tokens rotate; a reused one answers `refresh_token_reused`.
POST /control merges test knobs: refresh_barrier (hold refreshes until that many
arrived, 3 s at most), refresh_delay (seconds before a refresh answers),
hold_seconds (before a Codex reply), account (the account a sign-in gets),
aero (the account's billing: plan (Aerolite mid-period), aero, free, cancelling, past_due, out (no credits
left; 402 on chat), missing (404), error (500), rejected (401), echo (chat repeats the request's credential
in its error)).
Every request's headers and body are kept in /stats "raw" for leak checks."""
import base64, hashlib, json, os, sys, threading, time, urllib.parse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PORT = int(sys.argv[1])
TTL = int(sys.argv[2]) if len(sys.argv) > 2 else 3600
OPENAI_KEY_PREFIX, ANTHROPIC_KEY_PREFIX, CUSTOM_KEY = "sk-fake-openai-", "sk-ant-fake-", "custom-fake-key"
# PM_FAKE_RICH_MODELS=1: key model lists shaped like real ones (families with reasoning levels, enough rows to
# overflow a picker, a context variant), for on-screen checks. Off by default: tests assert the short lists.
RICH = os.environ.get("PM_FAKE_RICH_MODELS") == "1"
RICH_OPENAI = ["gpt-6-sol", "gpt-6-luna", "gpt-6-astra", "gpt-5.6", "gpt-5.5", "gpt-5.5-pro", "gpt-5.4", "gpt-5.4-pro",
               "gpt-5.3", "gpt-5.2", "o4-mini", "o3", "gpt-oss-120b", "text-embedding-fake"]
RICH_ANTHROPIC = [{"id": "claude-opus-5-5", "display_name": "Claude Opus 5.5"},
                  {"id": "claude-opus-5-5-1m", "display_name": "Claude Opus 5.5 (1M context)"},
                  {"id": "claude-sonnet-5-5", "display_name": "Claude Sonnet 5.5"}]
ACCOUNT = "acct-fake-1"
# The development credential Aero sends a test endpoint (never the person's session).
AERO_SESSION = os.environ.get("PM_FAKE_AERO_SESSION", "pm-aero-development")
# On-screen runs: Aero's chat goes on to emulate at this base (its Chat Completions), the rest stays here.
AERO_EMULATE = os.environ.get("PM_FAKE_AERO_EMULATE")
AERO_MODELS = [{"id": "anthropic/claude-opus-5.5", "name": "Claude Opus 5.5", "efforts": ["low", "medium", "high", "xhigh", "max"],
                "defaultEffort": "high", "default": True, "provider": "anthropic", "context": 200000},
               {"id": "openai/gpt-6-sol", "name": "GPT-6 Sol", "efforts": ["none", "low", "medium", "high", "xhigh"]},
               {"id": "google/gemini-3-pro", "name": "Gemini 3 Pro", "efforts": ["low", "high"]}]
AERO_RICH = AERO_MODELS + [{"id": "anthropic/claude-sonnet-5.5", "name": "Claude Sonnet 5.5", "efforts": ["low", "medium", "high"]},
                           {"id": "openai/gpt-6-luna", "name": "GPT-6 Luna", "efforts": ["low", "medium", "high"]},
                           {"id": "google/gemini-3-flash", "name": "Gemini 3 Flash"},
                           {"id": "xai/grok-5", "name": "Grok 5"}]
PLANS = [{"id": "aerolite", "name": "Aerolite", "priceCents": 999, "credits": 1000},
         {"id": "aero", "name": "Aero", "priceCents": 1999, "credits": 2500}]
# Credits spent each day of the period so far (Nov 25 to Dec 24; "today" is Dec 8).
SPENT = [40, 12, 0, 66, 18, 0, 0, 24, 9, 5, 0, 14, 0, 0]
RUNS_AERO = ("plan", "aero", "cancelling", "past_due", "out", "echo")


def billing(state):
    """GET /v2/billing as the contract shapes it, for a test state."""
    if state == "free":
        return {"plan": None, "status": None, "cancelAtPeriodEnd": False,
                "credits": {"included": 0, "used": 0, "remaining": 0}, "period": None, "daily": [], "plans": PLANS}
    offer = PLANS[1] if state == "aero" else PLANS[0]
    included = offer["credits"]
    spent = [c * 3 for c in SPENT] if state == "aero" else SPENT
    used = included if state == "out" else sum(spent)
    if state == "out":
        spent = spent[:-1] + [included - sum(spent[:-1])]
    days = [{"date": f"2026-{11 if i < 6 else 12}-{25 + i if i < 6 else i - 5:02d}", "credits": c} for i, c in enumerate(spent)]
    return {"plan": {"id": offer["id"], "name": offer["name"], "priceCents": offer["priceCents"], "interval": "month"},
            "status": "past_due" if state == "past_due" else "active", "cancelAtPeriodEnd": state == "cancelling",
            "credits": {"included": included, "used": used, "remaining": included - used},
            "period": {"start": "2026-11-25T00:00:00Z", "end": "2026-12-25T00:00:00Z"}, "daily": days, "plans": PLANS}


lock = threading.Lock()
codes, refresh, used, access = {}, {}, set(), {}
stats = {"refreshes": 0, "reused": 0, "logins": 0, "requests": [], "raw": []}
control = {"refresh_barrier": 0, "refresh_delay": 0, "hold_seconds": 0, "account": ACCOUNT, "aero": "plan"}
arrivals = threading.Condition()
arrived = [0]
counter = [0]


def b64(data):
    return base64.urlsafe_b64encode(data).rstrip(b"=").decode()


def jwt(claims):
    return f"{b64(b'{\"alg\":\"none\"}')}.{b64(json.dumps(claims).encode())}.fake"


def new_tokens(with_id, account=None):
    with lock:
        counter[0] += 1
        n = counter[0]
    now = int(time.time())
    account = account or control["account"]
    at = jwt({"exp": now + TTL, "n": n, "scope": "openid profile email offline_access"})
    rt = f"rt_fake_{n}"
    access[at] = (now + TTL, account)
    refresh[rt] = account
    out = {"access_token": at, "refresh_token": rt, "token_type": "Bearer", "expires_in": TTL}
    if with_id:
        out["id_token"] = jwt({"email": "owner@example.com", "exp": now + 3600,
                               "https://api.openai.com/auth": {"chatgpt_account_id": account, "chatgpt_plan_type": "plus"}})
    return out


def find_text(value, marker):
    if isinstance(value, str):
        i = value.find(marker)
        return value[i + len(marker):] if i >= 0 else None
    items = value.values() if isinstance(value, dict) else value if isinstance(value, list) else []
    for item in items:
        found = find_text(item, marker)
        if found is not None:
            return found
    return None


def demo(body):
    """('text', str) or ('tool', name, args)."""
    text = body.decode("utf-8", "replace")
    if "Aero demo edit" in text:
        return ("text", "Renamed the first layer to Aero demo edit.")
    request = json.loads(body or b"{}")
    header = find_text(request, "Project at session start\n") or find_text(request, "Project updates\n")
    if header:
        try:
            context = json.loads(header.splitlines()[0])
            revision, layer = context["composition"]["revision"], context["layers"][0]["id"]
            return ("tool", "edit", {"op": "commands", "revision": revision, "args": {"label": "Aero demo edit",
                    "commands": [{"type": "set_layer", "target": layer, "patch": {"name": "Aero demo edit"}}]}})
        except (ValueError, KeyError, IndexError, TypeError):
            pass
    return ("text", "Hello from the fake provider.")


def responses_sse(reply, body):
    rid = hashlib.sha256(body).hexdigest()[:12]
    out = []
    ev = lambda v: out.append(f"event: {v['type']}\ndata: {json.dumps(v)}\n\n")
    ev({"type": "response.created", "response": {"id": f"resp_{rid}", "status": "in_progress", "output": []}})
    if reply[0] == "text":
        item = {"type": "message", "id": f"msg_{rid}", "role": "assistant", "content": []}
        ev({"type": "response.output_item.added", "output_index": 0, "item": item})
        ev({"type": "response.output_text.delta", "output_index": 0, "content_index": 0, "item_id": item["id"], "delta": reply[1]})
        item = dict(item, status="completed", content=[{"type": "output_text", "text": reply[1], "annotations": []}])
    else:
        args = json.dumps(reply[2])
        item = {"type": "function_call", "id": f"fc_{rid}", "call_id": f"call_{rid}", "name": reply[1], "arguments": ""}
        ev({"type": "response.output_item.added", "output_index": 0, "item": item})
        ev({"type": "response.function_call_arguments.delta", "output_index": 0, "item_id": item["id"], "delta": args})
        item = dict(item, arguments=args, status="completed")
    ev({"type": "response.output_item.done", "output_index": 0, "item": item})
    ev({"type": "response.completed", "response": {"id": f"resp_{rid}", "status": "completed", "output": [item],
        "usage": {"input_tokens": 10, "output_tokens": 5, "total_tokens": 15}}})
    return "".join(out)


def chat_sse(reply, body):
    rid = hashlib.sha256(body).hexdigest()[:12]
    chunk = lambda delta, finish=None: "data: " + json.dumps({"id": f"chatcmpl-{rid}", "object": "chat.completion.chunk",
        "choices": [{"index": 0, "delta": delta, "finish_reason": finish}]}) + "\n\n"
    if reply[0] == "text":
        return chunk({"role": "assistant", "content": reply[1]}) + chunk({}, "stop") + "data: [DONE]\n\n"
    call = {"index": 0, "id": f"call_{rid}", "type": "function", "function": {"name": reply[1], "arguments": json.dumps(reply[2])}}
    return chunk({"role": "assistant", "tool_calls": [call]}) + chunk({}, "tool_calls") + "data: [DONE]\n\n"


def messages_sse(reply, body):
    """Anthropic Messages events; a tool reply is answered as text here."""
    rid = hashlib.sha256(body).hexdigest()[:12]
    text = reply[1] if reply[0] == "text" else "Hello from the fake provider."
    out = []
    ev = lambda v: out.append(f"event: {v['type']}\ndata: {json.dumps(v)}\n\n")
    ev({"type": "message_start", "message": {"id": f"msg_{rid}", "type": "message", "role": "assistant", "content": [],
        "model": json.loads(body or b"{}").get("model", ""), "usage": {"input_tokens": 1, "output_tokens": 0}}})
    ev({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}})
    ev({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": text}})
    ev({"type": "content_block_stop", "index": 0})
    ev({"type": "message_delta", "delta": {"stop_reason": "end_turn"}, "usage": {"output_tokens": 1}})
    ev({"type": "message_stop"})
    return "".join(out)


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def reply(self, status, body=b"", kind="application/json", location=None):
        if isinstance(body, (dict, list)):
            body = json.dumps(body).encode()
        elif isinstance(body, str):
            body = body.encode()
        self.send_response(status)
        self.send_header("Content-Type", kind)
        self.send_header("Content-Length", str(len(body)))
        if location:
            self.send_header("Location", location)
        self.end_headers()
        self.wfile.write(body)

    def body(self):
        body = self.rfile.read(int(self.headers.get("Content-Length") or 0))
        with lock:
            stats["raw"].append(f"{self.command} {self.path}\n{self.headers}\n{body.decode('utf-8', 'replace')}")
        return body

    def note(self):
        with lock:
            stats["requests"].append({"method": self.command, "path": self.path.split("?")[0],
                "authorization": (self.headers.get("Authorization") or "")[:16],
                "account": self.headers.get("chatgpt-account-id"), "x-api-key": bool(self.headers.get("x-api-key"))})

    def forward(self, url, body):
        import urllib.request
        request = urllib.request.Request(url, data=body, method="POST", headers={"content-type": "application/json"})
        with urllib.request.urlopen(request) as answer:
            self.send_response(answer.status)
            self.send_header("Content-Type", answer.headers.get("Content-Type", "text/event-stream"))
            self.end_headers()
            while chunk := answer.read1(4096):
                self.wfile.write(chunk)
                self.wfile.flush()

    def bearer(self):
        return (self.headers.get("Authorization") or "").removeprefix("Bearer ")

    def do_GET(self):
        self.note()
        with lock:
            stats["raw"].append(f"{self.command} {self.path}\n{self.headers}")
        url = urllib.parse.urlparse(self.path)
        q = dict(urllib.parse.parse_qsl(url.query))
        if url.path == "/stats":
            with lock:
                return self.reply(200, dict(stats, requests=stats["requests"][-50:]))
        if url.path == "/auth/oauth/authorize":
            # Approves at once, as a person in the browser would.
            code = f"code_{len(codes) + 1}"
            codes[code] = q.get("code_challenge", "")
            target = q["redirect_uri"] + "?" + urllib.parse.urlencode({"code": code, "state": q.get("state", "")})
            return self.reply(302, target + "\n", "text/plain", location=target)
        if url.path == "/openai/v1/models":
            if not self.bearer().startswith(OPENAI_KEY_PREFIX):
                return self.reply(401, {"error": {"message": "bad key"}})
            if RICH:
                return self.reply(200, {"object": "list", "data": [{"id": i} for i in RICH_OPENAI]})
            return self.reply(200, {"object": "list", "data": [{"id": "gpt-fake-key"}, {"id": "text-embedding-fake"}]})
        if url.path == "/anthropic/v1/models":
            if not (self.headers.get("x-api-key") or "").startswith(ANTHROPIC_KEY_PREFIX):
                return self.reply(401, {"error": {"message": "bad key"}})
            if RICH:
                return self.reply(200, {"data": RICH_ANTHROPIC, "has_more": False})
            return self.reply(200, {"data": [{"id": "claude-fake-opus", "display_name": "Claude Fake Opus"}], "has_more": False})
        if url.path.startswith("/aero/v2/"):
            if self.bearer() != AERO_SESSION:
                return self.reply(401, {"error": "unauthorized"})
            state = control["aero"]
            if url.path == "/aero/v2/billing":
                if state == "rejected":
                    return self.reply(401, {"error": "unauthorized"})
                if state == "error":
                    return self.reply(500, {"error": "internal"})
                if state == "missing":
                    return self.reply(404, {"error": "not found"})
                return self.reply(200, billing("plan" if state == "echo" else state))
            if url.path == "/aero/v2/aero/models" and state in RUNS_AERO:
                return self.reply(200, {"data": AERO_RICH if RICH else AERO_MODELS})
        if url.path == "/custom/v1/models":
            return self.reply(200, {"object": "list", "data": [{"id": "custom-fake-model"}]})
        self.reply(404, {"error": "not found"})

    def do_POST(self):
        self.note()
        path = urllib.parse.urlparse(self.path).path
        body = self.body()
        if path == "/control":
            control.update(json.loads(body or b"{}"))
            return self.reply(200, control)
        if path == "/auth/oauth/token":
            form = dict(urllib.parse.parse_qsl(body.decode())) if body[:1] != b"{" else json.loads(body)
            if form.get("grant_type") == "authorization_code":
                challenge = codes.pop(form.get("code"), None)
                verifier = form.get("code_verifier", "")
                if challenge is None or b64(hashlib.sha256(verifier.encode()).digest()) != challenge:
                    return self.reply(400, {"error": "invalid_grant"})
                with lock:
                    stats["logins"] += 1
                return self.reply(200, new_tokens(True))
            if form.get("grant_type") == "refresh_token":
                old = form.get("refresh_token")
                if control["refresh_barrier"]:
                    with arrivals:
                        arrived[0] += 1
                        arrivals.notify_all()
                        arrivals.wait_for(lambda: arrived[0] >= control["refresh_barrier"], timeout=3)
                with lock:
                    if old in used:
                        stats["reused"] += 1
                        return self.reply(401, {"error": "refresh_token_reused"})
                    account = refresh.pop(old, None)
                    if not account:
                        return self.reply(401, {"error": "invalid_grant"})
                    used.add(old)
                    stats["refreshes"] += 1
                fresh = new_tokens(False, account)
                time.sleep(control["refresh_delay"])
                return self.reply(200, fresh)
            return self.reply(400, {"error": "unsupported_grant_type"})
        if path == "/auth/oauth/revoke":
            refresh.pop(dict(urllib.parse.parse_qsl(body.decode())).get("token"), None)
            return self.reply(200)
        if path == "/chatgpt/backend-api/codex/responses":
            expires, account = access.get(self.bearer(), (0, None))
            if expires < time.time() or self.headers.get("chatgpt-account-id") != account:
                return self.reply(401, {"detail": "unauthorized"})
            time.sleep(control["hold_seconds"])
            return self.reply(200, responses_sse(demo(body), body), "text/event-stream")
        if path == "/anthropic/v1/messages":
            # Anthropic reads the key as x-api-key, with the API version.
            if (self.headers.get("Authorization") or not (self.headers.get("x-api-key") or "").startswith(ANTHROPIC_KEY_PREFIX)
                    or not self.headers.get("anthropic-version")):
                return self.reply(401, {"type": "error", "error": {"type": "authentication_error", "message": "bad key"}})
            return self.reply(200, messages_sse(demo(body), body), "text/event-stream")
        if path in ("/aero/v2/billing/checkout", "/aero/v2/billing/portal"):
            if self.bearer() != AERO_SESSION:
                return self.reply(401, {"error": {"code": "unauthorized"}})
            if path.endswith("/portal"):
                return self.reply(200, {"url": "https://billing.stripe.com/p/session/test_fake"})
            if control["aero"] != "free":
                return self.reply(409, {"error": {"code": "has_plan"}})
            plan = json.loads(body or b"{}").get("plan")
            if plan not in ("aerolite", "aero"):
                return self.reply(400, {"error": {"code": "bad_plan"}})
            return self.reply(200, {"url": f"https://checkout.stripe.com/c/pay/cs_test_fake_{plan}"})
        if path == "/aero/v2/aero/chat/completions":
            if control["aero"] == "echo":
                # A careless backend that repeats the request's credential in its error.
                return self.reply(400, {"error": {"message": f"bad request with {self.headers.get('Authorization')}",
                                                  "code": "invalid_request"}})
            if self.bearer() != AERO_SESSION:
                return self.reply(401, {"error": {"code": "unauthorized"}})
            if control["aero"] == "out":
                return self.reply(402, {"error": {"code": "credits_exhausted"}})
            if control["aero"] not in RUNS_AERO:
                return self.reply(403, {"error": {"code": "no_plan"}})
            time.sleep(control["hold_seconds"])
            if AERO_EMULATE:
                return self.forward(f"{AERO_EMULATE}/v1/chat/completions", body)
            return self.reply(200, chat_sse(demo(body), body), "text/event-stream")
        keys = {"/openai/v1/chat/completions": lambda: self.bearer().startswith(OPENAI_KEY_PREFIX),
                "/custom/v1/chat/completions": lambda: self.bearer() == CUSTOM_KEY}
        if path == "/custom/v1/chat/completions" and self.bearer().startswith("custom-echo-"):
            # A careless server that repeats the request's credential in its error.
            return self.reply(400, {"error": {"message": f"bad request with {self.headers.get('Authorization')}",
                                              "code": "invalid_request"}})
        if path in keys:
            if not keys[path]():
                return self.reply(401, {"error": {"message": "bad key"}})
            return self.reply(200, chat_sse(demo(body), body), "text/event-stream")
        self.reply(404, {"error": "not found"})


ThreadingHTTPServer(("127.0.0.1", PORT), Handler).serve_forever()
