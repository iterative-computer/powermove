"""Synthetic provider peer. No accounts, network or user files."""
import json
import sys
from pathlib import Path
import time

def emit(value):
    print(json.dumps(value), flush=True)

mode = sys.argv[1]
if mode == 'idle':
    for line in sys.stdin:
        pass
elif mode == 'claude-steer':
    emit({'type':'assistant','message':{'content':[{'type':'text','text':'First answer'}]}})
    direction = json.loads(sys.stdin.readline())
    assert direction['type'] == 'user'
    assert direction['message']['content'][0]['text'] == 'Make it blue'
    emit({'type':'result','result':'First answer'})
    assert sys.stdin.read() == ''
    time.sleep(2.2)  # A queued turn outlives the ordinary two-second reap grace.
    emit({'type':'result','result':'Blue answer'})
elif mode == 'claude-tool-steer':
    emit({'type':'assistant','message':{'content':[{'type':'tool_use','id':'tool','name':'Read','input':{}}]}})
    direction = json.loads(sys.stdin.readline())
    assert direction['type'] == 'user'
    assert 'Make it blue' in json.dumps(direction)
    emit({'type':'result','result':'Blue answer'})
elif mode == 'claude-double-steer':
    for expected in ['Make it blue', 'Keep it short']:
        direction = json.loads(sys.stdin.readline())
        assert direction['type'] == 'user'
        assert direction['message']['content'][0]['text'] == expected
    emit({'type':'result','result':'Short blue answer'})
    assert sys.stdin.read() == ''
elif mode == 'claude':
    emit({'type': 'control_request', 'request_id': 'control', 'request': {
        'subtype': 'can_use_tool', 'tool_name': 'AskUserQuestion', 'tool_use_id': 'question',
        'input': {'questions': [{'question': 'Choose a style?', 'options': [{'label': 'Soft'}]}]}}})
    reply = json.loads(sys.stdin.readline())
    assert reply['response']['response']['updatedInput']['answers'] == {'Choose a style?': 'Soft'}
    emit({'type': 'assistant', 'session_id': 'session', 'message': {'content': [{'type': 'text', 'text': '**Done**'}]}})
    emit({'type': 'result', 'result': '**Done**'})
elif mode == 'codex':
    for line in sys.stdin:
        request = json.loads(line)
        method = request.get('method')
        if method == 'initialize':
            emit({'id': request['id'], 'result': {}})
        elif method == 'thread/start':
            emit({'id': request['id'], 'result': {'thread': {'id': 'thread'}}})
        elif method == 'turn/start':
            emit({'id': request['id'], 'result': {'turn': {'id': 'turn'}}})
            emit({'method': 'turn/started', 'params': {'threadId': 'thread', 'turn': {'id': 'turn'}}})
            emit({'method': 'item/started', 'params': {'threadId': 'thread', 'turnId': 'turn', 'item': {'type': 'commandExecution', 'id': 'work', 'command': 'synthetic'}}})
        elif method == 'turn/interrupt':
            Path(sys.argv[2]).write_text('interrupt received', encoding='utf-8')
            # Explicit causal barrier: the test releases acknowledgement only
            # after inspecting the panel's stopping (nonterminal) publication.
            deadline = time.monotonic() + 5
            while not Path(sys.argv[2] + '.release').exists():
                if time.monotonic() > deadline:
                    raise SystemExit('interrupt barrier timed out')
                time.sleep(0.002)
            emit({'id': request['id'], 'result': {}})
else:
    raise SystemExit(7)
