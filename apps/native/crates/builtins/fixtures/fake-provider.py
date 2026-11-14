#!/usr/bin/python3
"""Synthetic Codex and Claude peer over real pipes. No account or network."""
import json
import os
import sys
import threading
import time

LOCK = threading.Lock()
STREAMING = threading.Event()


def emit(value):
    with LOCK:
        print(json.dumps(value), flush=True)


def signed_in():
    if os.path.exists(flag('out')):
        return False
    return os.environ.get('PM_FAKE_PROVIDER') != 'signed-out' or os.path.exists(flag('in'))


def flag(state):
    home = os.environ.get('CODEX_HOME') or os.environ['CLAUDE_CONFIG_DIR']
    return os.path.join(home, 'fake-signed-' + state)


def sign_in():
    open(flag('in'), 'w').close()
    if os.path.exists(flag('out')):
        os.remove(flag('out'))


def sign_out():
    if os.path.exists(flag('in')):
        os.remove(flag('in'))
    open(flag('out'), 'w').close()


def stream(turn):
    """A token every few milliseconds until interrupted, as a long reply is."""
    STREAMING.set()
    while STREAMING.is_set():
        emit({'method': 'item/agentMessage/delta', 'params': {**turn, 'itemId': 'answer', 'delta': 'token '}})
        time.sleep(0.003)


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
        if request.get('id') == 900 and 'result' in request:
            # The held question was answered.
            text = 'Answered: ' + json.dumps(request['result'].get('answers', {}), sort_keys=True)
            emit({'method': 'item/agentMessage/delta', 'params': {**turn, 'itemId': 'answer', 'delta': text}})
            emit({'method': 'item/completed', 'params': {**turn, 'item': {'id': 'answer', 'type': 'agentMessage', 'text': text}}})
            emit({'method': 'turn/completed', 'params': {'threadId': 'thread', 'turn': {'id': 'turn', 'status': 'completed', 'error': None}}})
        elif method == 'initialize':
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
            # The default model, one with other efforts and one with none.
            emit({'id': request['id'], 'result': {'data': [{'model': 'fixture-5', 'displayName': 'Fixture 5', 'hidden': False,
                'isDefault': True, 'defaultReasoningEffort': 'high',
                'supportedReasoningEfforts': [{'reasoningEffort': 'low'}, {'reasoningEffort': 'high'}]},
                {'model': 'fixture-6', 'displayName': 'Fixture 6', 'hidden': False, 'isDefault': False,
                 'defaultReasoningEffort': 'medium',
                 'supportedReasoningEfforts': [{'reasoningEffort': 'low'}, {'reasoningEffort': 'medium'},
                                               {'reasoningEffort': 'xhigh'}]},
                {'model': 'fixture-plain', 'displayName': 'Fixture Plain', 'hidden': False, 'isDefault': False,
                 'supportedReasoningEfforts': []}]}})
        elif method == 'thread/start':
            emit({'id': request['id'], 'result': {'thread': {'id': 'thread'}}})
        elif method == 'turn/start':
            emit({'id': request['id'], 'result': {'turn': {'id': 'turn', 'status': 'inProgress'}}})
            emit({'method': 'turn/started', 'params': {'threadId': 'thread', 'turn': {'id': 'turn'}}})
            last = request['params']['input'][0]['text'].split('\n')[-1]
            if last.startswith('Secret:'):
                emit({'id': 900, 'method': 'item/tool/requestUserInput', 'params': {**turn, 'itemId': 'q1', 'isBlocking': True,
                    'questions': [{'id': 'pin', 'question': 'What is the PIN?', 'isSecret': True}]}})
            elif mode == 'stream':
                threading.Thread(target=stream, args=(turn,), daemon=True).start()
            elif mode != 'hold' and not request['params']['input'][0]['text'].split('\n')[-1].startswith('Hold:'):
                if mode == 'slow':
                    time.sleep(float(os.environ.get('PM_FAKE_DELAY', '0.4')))
                elif request['params']['input'][0]['text'].split('\n')[-1].startswith('Slow:'):
                    time.sleep(12)
                text = 'Saw: ' + request['params']['input'][0]['text'].split('\n')[-1]
                emit({'method': 'item/agentMessage/delta', 'params': {**turn, 'itemId': 'answer', 'delta': text}})
                emit({'method': 'item/completed', 'params': {**turn, 'item': {'id': 'answer', 'type': 'agentMessage', 'text': text}}})
                emit({'method': 'turn/completed', 'params': {'threadId': 'thread', 'turn': {'id': 'turn', 'status': 'completed', 'error': None}}})
        elif method == 'turn/interrupt':
            STREAMING.clear()
            emit({'id': request['id'], 'result': {}})
            emit({'method': 'turn/completed', 'params': {'threadId': 'thread', 'turn': {'id': 'turn', 'status': 'interrupted', 'error': None}}})


def codex_exec():
    """`codex exec --json` (project access): the prompt is the last argument,
    events are JSON lines, the reply also goes to --output-last-message."""
    last = sys.argv[-1].split('\n')[-1]
    output = sys.argv[sys.argv.index('--output-last-message') + 1]
    emit({'type': 'thread.started', 'thread_id': 'exec-thread'})
    emit({'type': 'turn.started'})
    if os.environ.get('PM_FAKE_PROVIDER') == 'hold' or last.startswith('Hold:'):
        sys.stdin.read()
        return
    emit({'type': 'item.started', 'item': {'id': 'cmd', 'type': 'command_execution', 'command': 'ls -la', 'status': 'in_progress'}})
    emit({'type': 'item.completed', 'item': {'id': 'cmd', 'type': 'command_execution', 'command': 'ls -la',
        'aggregated_output': 'project.pmv', 'exit_code': 0, 'status': 'completed'}})
    reply = json.dumps({'reply': 'Saw: ' + last})
    emit({'type': 'item.completed', 'item': {'id': 'answer', 'type': 'agent_message', 'text': reply}})
    with open(output, 'w') as file:
        file.write(reply)
    emit({'type': 'turn.completed', 'usage': {'input_tokens': 1, 'output_tokens': 1}})


if os.environ.get('PM_FAKE_RECORD') and '--allowedTools' in sys.argv:
    with open(os.environ['PM_FAKE_RECORD'], 'w') as record:
        json.dump(sys.argv, record)
if sys.argv[1:2] == ['auth']:
    claude_auth(sys.argv[2])
elif '--setting-sources' in sys.argv:
    claude_models()
elif os.path.basename(sys.argv[0]) == 'claude' or '--print' in sys.argv:
    claude()
elif 'exec' in sys.argv:
    codex_exec()
else:
    codex()
