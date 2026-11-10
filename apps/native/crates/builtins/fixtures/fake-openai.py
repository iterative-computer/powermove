#!/usr/bin/python3
"""Synthetic OpenAI-compatible chat server on 127.0.0.1. No account or network.

Prints its port, then answers POST /v1/chat/completions with a streamed reply:
- after a tool result: "Tool said: <its text>" in several fragments;
- a prompt containing "slow": fragments one per second (for Stop);
- a prompt containing "tool" with tools offered: one call to the first tool;
- otherwise: "Saw: <prompt>".
GET /v1/models lists "fixture-model". Each request body is appended to
$PM_FAKE_OPENAI_RECORD (one JSON line) when set.
"""
import http.server
import json
import os
import sys
import time


def chunk(delta, finish=None):
    return {"choices": [{"index": 0, "delta": delta, "finish_reason": finish}]}


class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def do_GET(self):
        if self.path.rstrip('/').endswith('/models'):
            body = json.dumps({"data": [{"id": "fixture-model"}]}).encode()
            self.send_response(200)
            self.send_header('Content-Type', 'application/json')
            self.send_header('Content-Length', str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        else:
            self.send_error(404)

    def do_POST(self):
        if not self.path.rstrip('/').endswith('/chat/completions'):
            self.send_error(404)
            return
        length = int(self.headers.get('Content-Length', '0'))
        request = json.loads(self.rfile.read(length) or b'{}')
        record = os.environ.get('PM_FAKE_OPENAI_RECORD')
        if record:
            with open(record, 'a') as out:
                out.write(json.dumps({"auth": self.headers.get('Authorization'), "body": request}) + '\n')
        if not request.get('stream'):
            body = json.dumps({"choices": [{"message": {"role": "assistant", "content": "OK"}}]}).encode()
            self.send_response(200)
            self.send_header('Content-Type', 'application/json')
            self.send_header('Content-Length', str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        self.send_response(200)
        self.send_header('Content-Type', 'text/event-stream')
        self.end_headers()
        messages = request.get('messages', [])
        last = messages[-1] if messages else {}
        prompt = next((m['content'] for m in messages if m.get('role') == 'user'), '')
        if isinstance(prompt, list):
            prompt = ' '.join(p.get('text', '') for p in prompt if isinstance(p, dict))
        events, delay = [], 0.02
        if last.get('role') == 'tool':
            said = json.loads(last['content'])
            text = ' '.join(c.get('text', '') for c in said.get('content', []))[:80]
            for part in ['Tool ', 'said: ', text]:
                events.append(chunk({"content": part}))
            events.append(chunk({}, 'stop'))
        elif 'slow' in prompt:
            delay = 1.0
            for n in range(60):
                events.append(chunk({"content": f"token{n} "}))
            events.append(chunk({}, 'stop'))
        elif 'tool' in prompt and request.get('tools'):
            name = request['tools'][0]['function']['name']
            events.append(chunk({"tool_calls": [{"index": 0, "id": "call_1", "type": "function",
                                                  "function": {"name": name, "arguments": ""}}]}))
            events.append(chunk({"tool_calls": [{"index": 0, "function": {"arguments": "{}"}}]}))
            events.append(chunk({}, 'tool_calls'))
        else:
            words = ('Saw: ' + prompt.split('USER REQUEST\n')[-1]).split(' ')
            for i, word in enumerate(words):
                events.append(chunk({"content": word if i == 0 else ' ' + word}))
            events.append(chunk({}, 'stop'))
        try:
            for event in events:
                self.wfile.write(f"data: {json.dumps(event)}\r\n\r\n".encode())
                self.wfile.flush()
                time.sleep(delay)
            self.wfile.write(b"data: [DONE]\n\n")
            self.wfile.flush()
        except (BrokenPipeError, ConnectionResetError):
            pass


server = http.server.ThreadingHTTPServer(('127.0.0.1', int(sys.argv[1]) if len(sys.argv) > 1 else 0), Handler)
print(server.server_address[1], flush=True)
server.serve_forever()
