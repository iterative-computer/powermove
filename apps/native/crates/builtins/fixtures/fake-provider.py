#!/usr/bin/python3
"""Synthetic Codex and Claude peer over real pipes. No account or network."""
import json
import os
import sys


def emit(value):
    print(json.dumps(value), flush=True)


def claude():
    json.loads(sys.stdin.readline())
    emit({'type': 'control_request', 'request_id': 'control', 'request': {
        'subtype': 'can_use_tool', 'tool_name': 'AskUserQuestion', 'tool_use_id': 'question',
        'input': {'questions': [{'question': 'Choose a style?', 'options': [{'label': 'Soft'}]}]}}})
    reply = json.loads(sys.stdin.readline())
    answer = reply['response']['response']['updatedInput']['answers']['Choose a style?']
    emit({'type': 'assistant', 'session_id': 'session', 'message': {
        'content': [{'type': 'text', 'text': 'Using ' + answer}]}})
    emit({'type': 'result', 'structured_output': {'reply': 'Using ' + answer}})


def codex():
    mode = os.environ.get('PM_FAKE_PROVIDER', 'reply')
    turn = {'threadId': 'thread', 'turnId': 'turn'}
    for line in sys.stdin:
        request = json.loads(line)
        method = request.get('method')
        if method == 'initialize':
            emit({'id': request['id'], 'result': {}})
        elif method == 'thread/start':
            emit({'id': request['id'], 'result': {'thread': {'id': 'thread'}}})
        elif method == 'turn/start':
            emit({'id': request['id'], 'result': {'turn': {'id': 'turn', 'status': 'inProgress'}}})
            emit({'method': 'turn/started', 'params': {'threadId': 'thread', 'turn': {'id': 'turn'}}})
            if mode == 'reply':
                text = 'Saw: ' + request['params']['input'][0]['text'].split('\n')[-1]
                emit({'method': 'item/agentMessage/delta', 'params': {**turn, 'itemId': 'answer', 'delta': text}})
                emit({'method': 'item/completed', 'params': {**turn, 'item': {'id': 'answer', 'type': 'agentMessage', 'text': text}}})
                emit({'method': 'turn/completed', 'params': {'threadId': 'thread', 'turn': {'id': 'turn', 'status': 'completed', 'error': None}}})
        elif method == 'turn/interrupt':
            emit({'id': request['id'], 'result': {}})
            emit({'method': 'turn/completed', 'params': {'threadId': 'thread', 'turn': {'id': 'turn', 'status': 'interrupted', 'error': None}}})


if os.path.basename(sys.argv[0]) == 'claude' or '--print' in sys.argv:
    claude()
else:
    codex()
