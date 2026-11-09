import { createInterface } from 'node:readline';

/** Speak the same App Server protocol as the production Codex transport.
 * Keep status, thread configuration and turn completion on the real IPC path. */
export async function serveCodexFixture(complete) {
  if (process.argv.includes('--version')) {
    process.stdout.write('codex-cli 0.159.1-e2e\n');
    return;
  }
  const threads = new Map();
  let nextThread = 0;
  let nextTurn = 0;
  const send = message => process.stdout.write(JSON.stringify(message) + '\n');
  for await (const line of createInterface({ input: process.stdin })) {
    const message = JSON.parse(line);
    if (message.id == null) continue;
    const reply = result => send({ id: message.id, result });
    switch (message.method) {
      case 'initialize':
        reply({ userAgent: 'powermove-e2e', codexHome: '/tmp/powermove-e2e-codex', platformFamily: 'unix', platformOs: 'macos' });
        break;
      case 'account/read':
        reply({ account: { type: 'chatgpt', email: 'motion@example.com', planType: 'pro' }, requiresOpenaiAuth: true });
        break;
      case 'thread/start':
      case 'thread/resume': {
        const id = message.params.threadId ?? `e2e-thread-${++nextThread}`;
        threads.set(id, message.params);
        reply({ thread: { id } });
        break;
      }
      case 'turn/start': {
        const turnId = `e2e-turn-${++nextTurn}`;
        const scope = { threadId: message.params.threadId, turnId };
        reply({ turn: { id: turnId, status: 'inProgress' } });
        send({ method: 'turn/started', params: { ...scope, turn: { id: turnId, status: 'inProgress' } } });
        try {
          const result = await complete(threads.get(scope.threadId), message.params);
          send({ method: 'item/completed', params: { ...scope, item: { id: `answer-${turnId}`, type: 'agentMessage', text: JSON.stringify(result) } } });
          send({ method: 'turn/completed', params: { ...scope, turn: { id: turnId, status: 'completed' } } });
        } catch (error) {
          send({ method: 'turn/completed', params: { ...scope, turn: { id: turnId, status: 'failed', error: { message: String(error) } } } });
        }
        break;
      }
      default:
        reply({});
    }
  }
}
