#!/usr/bin/env python3
"""A fake `codex` for Settings › Providers tests and on-screen passes: the
App Server account methods Powermove calls, with Codex 0.169's keyring store
(one generic password, in the lane's test keychain file PM_AGENT_KEYCHAIN,
service PM_CODEX_KEYRING_SERVICE, account
`cli|<first 16 hex of sha256(realpath(CODEX_HOME))>`, Codex's auth.json
document). Sign-in follows the Codex ChatGPT route against the fake issuer
PM_FAKE_AUTH (authorize, loopback /auth/callback, code exchange with PKCE);
`account/read` with `refreshToken` refreshes when the access token is within
five minutes of expiring, the only refresher, as Codex is. Items are made
with `security -A`, only in that file and under the lane's service."""
import base64, hashlib, json, os, secrets, subprocess, sys, threading, time, urllib.parse, urllib.request
from http.server import BaseHTTPRequestHandler, HTTPServer

if os.environ.get("PM_FAKE_CODEX_LOG"):
    sys.stderr = open(os.environ["PM_FAKE_CODEX_LOG"], "a")
SERVICE = os.environ.get("PM_CODEX_KEYRING_SERVICE", "")
ISSUER = os.environ.get("PM_FAKE_AUTH", "")
if not SERVICE or SERVICE == "Codex Auth":
    sys.exit("fake codex: set a lane-only PM_CODEX_KEYRING_SERVICE")
HOME = os.path.realpath(os.environ["CODEX_HOME"])
KEY = "cli|" + hashlib.sha256(HOME.encode()).hexdigest()[:16]
out_lock = threading.Lock()
# Only the lane's own test keychain file, named on every call: never the
# login keychain or the search list, and a call that would wait on a prompt
# times out instead.
KEYCHAIN = os.environ.get("PM_AGENT_KEYCHAIN", "")
if not KEYCHAIN.endswith(".keychain-db") or not os.path.isfile(KEYCHAIN):
    sys.exit("fake codex: set PM_AGENT_KEYCHAIN to the lane's test keychain file")


def security(*args, check=False):
    return subprocess.run(["/usr/bin/security", *args, KEYCHAIN], capture_output=True, text=True,
                          stdin=subprocess.DEVNULL, timeout=5, check=check)


def send(value):
    with out_lock:
        sys.stdout.write(json.dumps(value) + "\n")
        sys.stdout.flush()


def item():
    found = security("find-generic-password", "-s", SERVICE, "-a", KEY, "-w")
    return json.loads(found.stdout) if found.returncode == 0 and found.stdout.strip() else None


def save(document):
    # Never update in place (`-U` can ask for access): delete, then add.
    security("delete-generic-password", "-s", SERVICE, "-a", KEY)
    security("add-generic-password", "-A", "-s", SERVICE, "-a", KEY, "-w", json.dumps(document, separators=(",", ":")), check=True)


def delete():
    security("delete-generic-password", "-s", SERVICE, "-a", KEY)


def claims(token):
    part = token.split(".")[1]
    return json.loads(base64.urlsafe_b64decode(part + "=" * (-len(part) % 4)))


def post(path, form):
    request = urllib.request.Request(ISSUER + path, data=urllib.parse.urlencode(form).encode(), method="POST")
    with urllib.request.urlopen(request, timeout=10) as answer:
        return json.loads(answer.read() or b"{}")


def refresh_if_stale(force=False):
    document = item()
    tokens = (document or {}).get("tokens")
    if not tokens:
        return
    if not force and claims(tokens["access_token"]).get("exp", 0) > time.time() + 300:
        return
    fresh = post("/oauth/token", {"grant_type": "refresh_token", "refresh_token": tokens["refresh_token"],
                                  "client_id": "app_EMoamEEZ73f0CkXaXp7hrann"})
    tokens.update(access_token=fresh["access_token"], refresh_token=fresh["refresh_token"])
    document["last_refresh"] = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
    save(document)


def account():
    document = item()
    if not document:
        return {"account": None, "requiresOpenaiAuth": True}
    if document.get("tokens"):
        auth = claims(document["tokens"]["id_token"]).get("https://api.openai.com/auth", {})
        return {"account": {"type": "chatgpt", "email": claims(document["tokens"]["id_token"]).get("email"),
                            "planType": auth.get("chatgpt_plan_type")}}
    return {"account": {"type": "apiKey"}}


def login(login_id):
    verifier = secrets.token_urlsafe(32)
    challenge = base64.urlsafe_b64encode(hashlib.sha256(verifier.encode()).digest()).rstrip(b"=").decode()
    state = secrets.token_urlsafe(16)
    done = {}

    class Callback(BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass

        def do_GET(self):
            url = urllib.parse.urlparse(self.path)
            q = dict(urllib.parse.parse_qsl(url.query))
            ok = url.path == "/auth/callback" and q.get("state") == state
            self.send_response(200 if ok else 400)
            self.end_headers()
            if ok:
                done.update(q)

    server = HTTPServer(("127.0.0.1", 0), Callback)
    redirect = f"http://127.0.0.1:{server.server_port}/auth/callback"
    url = ISSUER + "/oauth/authorize?" + urllib.parse.urlencode({"response_type": "code", "client_id": "app_EMoamEEZ73f0CkXaXp7hrann",
        "redirect_uri": redirect, "scope": "openid profile email offline_access", "code_challenge": challenge,
        "code_challenge_method": "S256", "state": state})

    def wait():
        while "code" not in done:
            server.handle_request()
        tokens = post("/oauth/token", {"grant_type": "authorization_code", "code": done["code"], "redirect_uri": redirect,
                                       "client_id": "app_EMoamEEZ73f0CkXaXp7hrann", "code_verifier": verifier})
        save({"auth_mode": "chatgpt", "OPENAI_API_KEY": None, "tokens": {"id_token": tokens["id_token"],
              "access_token": tokens["access_token"], "refresh_token": tokens["refresh_token"],
              "account_id": claims(tokens["id_token"])["https://api.openai.com/auth"]["chatgpt_account_id"]},
              "last_refresh": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())})
        send({"method": "account/login/completed", "params": {"loginId": login_id, "success": True}})
        send({"method": "account/updated", "params": {}})

    threading.Thread(target=wait, daemon=True).start()
    return url


def main():
    if sys.argv[1:2] == ["--version"]:
        print("codex-cli 0.169.0")
        return
    if sys.argv[1:2] != ["app-server"]:
        sys.exit("fake codex: only app-server")
    for line in sys.stdin:
        message = json.loads(line)
        method, params, ident = message.get("method"), message.get("params") or {}, message.get("id")
        if ident is None:
            continue
        try:
            if method == "initialize":
                result = {"userAgent": "fake-codex/0.169.0"}
            elif method == "account/read":
                if params.get("refreshToken"):
                    refresh_if_stale()
                result = account()
            elif method == "account/login/start" and params.get("type") == "chatgpt":
                result = {"type": "chatgpt", "loginId": "login-1", "authUrl": login("login-1")}
            elif method == "account/login/start" and params.get("type") == "apiKey":
                save({"auth_mode": "apikey", "OPENAI_API_KEY": params["apiKey"]})
                result = {"type": "apiKey"}
            elif method == "account/login/cancel":
                result = {}
            elif method == "account/logout":
                delete()
                result = {}
            elif method == "model/list":
                result = {"data": [{"id": "gpt-fake-sol", "model": "gpt-fake-sol", "displayName": "GPT Fake Sol", "isDefault": True,
                                    "supportedReasoningEfforts": [{"reasoningEffort": e} for e in ("low", "medium", "high")],
                                    "defaultReasoningEffort": "medium"}], "nextCursor": None}
            else:
                send({"id": ident, "error": {"code": -32601, "message": f"fake codex: {method}"}})
                continue
            send({"id": ident, "result": result})
        except Exception as error:  # noqa: BLE001 - a fake reports, never crashes the peer
            send({"id": ident, "error": {"code": -32000, "message": str(error)}})


main()
