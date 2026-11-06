#!/usr/bin/python3
"""Synthetic Codex and Claude peer over real pipes. No account or network."""
import json
import os
import sys


def emit(value):
    print(json.dumps(value), flush=True)


def signed_in():
    return os.environ.get('PM_FAKE_PROVIDER') != 'signed-out' or os.path.exists(flag())


def flag():
    home = os.environ.get('CODEX_HOME') or os.environ['CLAUDE_CONFIG_DIR']
    return os.path.join(home, 'fake-signed-in')


def sign_in():
    open(flag(), 'w').close()


def sign_out():
    if os.path.exists(flag()):
        os.remove(flag())


def claude_auth(step):
    if step == 'status':
        emit({'loggedIn': signed_in(), 'email': 'user@example.invalid', 'subscriptionType': 'pro'})
    elif step == 'login':
        sign_in()
    else:
        sign_out()


def claude_models():
    json.loads(sys.stdin.readline())
    emit({'type': 'control_response', 'response': {'subtype': 'success', 'request_id': 'models', 'response': {'models': [
        {'value': 'fixture', 'displayName': 'Fixture', 'supportedEffortLevels': ['low', 'high']}]}}})
    sys.stdin.read()


def claude_control(subtype_request):
    emit({'type': 'control_request', 'request_id': 'control', 'request': {'subtype': 'can_use_tool', **subtype_request}})
    return json.loads(sys.stdin.readline())['response']['response']


def claude():
    json.loads(sys.stdin.readline())
    if os.environ.get('PM_FAKE_PROVIDER') == 'approval':
        decision = claude_control({'tool_name': 'Bash', 'tool_use_id': 'tool', 'input': {'command': 'touch outside'}})
        answer = decision['behavior']
    else:
        decision = claude_control({'tool_name': 'AskUserQuestion', 'tool_use_id': 'question', 'input': {
            'questions': [{'question': 'Choose a style?', 'options': [{'label': 'Soft'}]}]}})
        answer = decision['updatedInput']['answers']['Choose a style?']
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
        elif method == 'account/read':
            account = {'type': 'chatgpt', 'email': 'user@example.invalid', 'planType': 'plus'}
            emit({'id': request['id'], 'result': {'account': account if signed_in() else None}})
        elif method == 'account/login/start':
            emit({'id': request['id'], 'result': {'type': 'chatgpt', 'loginId': 'login', 'authUrl': 'https://chatgpt.com/auth'}})
            sign_in()
            emit({'method': 'account/login/completed', 'params': {'loginId': 'login', 'success': True}})
        elif method == 'account/logout':
            sign_out()
            emit({'id': request['id'], 'result': {}})
        elif method == 'model/list':
            emit({'id': request['id'], 'result': {'data': [{'model': 'fixture-5', 'displayName': 'Fixture 5', 'hidden': False,
                'supportedReasoningEfforts': [{'reasoningEffort': 'low'}, {'reasoningEffort': 'high'}]}]}})
        elif method == 'thread/start':
            emit({'id': request['id'], 'result': {'thread': {'id': 'thread'}}})
        elif method == 'turn/start':
            emit({'id': request['id'], 'result': {'turn': {'id': 'turn', 'status': 'inProgress'}}})
            emit({'method': 'turn/started', 'params': {'threadId': 'thread', 'turn': {'id': 'turn'}}})
            if mode != 'hold':
                text = 'Saw: ' + request['params']['input'][0]['text'].split('\n')[-1]
                emit({'method': 'item/agentMessage/delta', 'params': {**turn, 'itemId': 'answer', 'delta': text}})
                emit({'method': 'item/completed', 'params': {**turn, 'item': {'id': 'answer', 'type': 'agentMessage', 'text': text}}})
                emit({'method': 'turn/completed', 'params': {'threadId': 'thread', 'turn': {'id': 'turn', 'status': 'completed', 'error': None}}})
        elif method == 'turn/interrupt':
            emit({'id': request['id'], 'result': {}})
            emit({'method': 'turn/completed', 'params': {'threadId': 'thread', 'turn': {'id': 'turn', 'status': 'interrupted', 'error': None}}})


if os.environ.get('PM_FAKE_RECORD') and '--allowedTools' in sys.argv:
    with open(os.environ['PM_FAKE_RECORD'], 'w') as record:
        json.dump(sys.argv, record)
if sys.argv[1:2] == ['auth']:
    claude_auth(sys.argv[2])
elif '--setting-sources' in sys.argv:
    claude_models()
elif os.path.basename(sys.argv[0]) == 'claude' or '--print' in sys.argv:
    claude()
else:
    codex()
