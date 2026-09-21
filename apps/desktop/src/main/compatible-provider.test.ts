import { afterEach, expect, it, vi } from 'vitest';
import { mkdtemp, readFile, rm, access } from 'node:fs/promises';
import path from 'node:path';
import os from 'node:os';
vi.mock('electron', () => ({ safeStorage: { isEncryptionAvailable: () => true, encryptString: (value: string) => Buffer.from('encrypted:' + value), decryptString: (value: Buffer) => value.toString().slice(10) } }));
import { CompatibleProvider, readCompletion } from './compatible-provider';
import { providerUrl } from '../shared/compatible-provider';
import { restoreExtensionChangeSet } from './codex/change-history';
import { prepareAgentWorkspace } from './codex/workspace';
import { agentResultSchema } from './codex/instructions';
const directories: string[] = [];
afterEach(async () => { for (const dir of directories.splice(0)) await rm(dir, { recursive: true, force: true }); });
const event = (delta: any, finish_reason: any = null) => `data: ${JSON.stringify({ choices: [{ delta, finish_reason }] })}\r\n\r\n`;
it('authors and publishes a real effect through API tools, collects artifacts and preserves recovery history', async () => {
  const directory = await mkdtemp(path.join(os.tmpdir(), 'pm-api-authoring-')); directories.push(directory);
  const sample = path.resolve('docs/samples/gradient-tint');
  const manifest = await readFile(path.join(sample, 'manifest.json'), 'utf8');
  const source = await readFile(path.join(sample, 'index.ts'), 'utf8');
  let stage = '', turn = 0;
  const requests: any[] = [];
  const complete = { summary: 'Created Gradient Tint.', commands: [], artifacts: [], extensions: [{ id: 'gradient-tint', action: 'created', summary: 'Keyframeable gradient tint' }], notes: [], externalActions: [] };
  const fetcher = vi.fn(async (_url: any, options: any) => {
    if (options.method === 'GET') return new Response(null, { status: 404 });
    const body = JSON.parse(options.body);
    if (!body.stream) return Response.json({ choices: [{ message: { content: 'OK' } }] });
    requests.push(body);
    const calls = [
      ['read_file', { path: 'powermove-api/api.ts' }],
      ['write_file', { path: path.join(stage, 'gradient-tint/manifest.json'), text: manifest }],
      ['write_file', { path: path.join(stage, 'gradient-tint/index.ts'), text: source }],
      ['compile_extension', { id: 'gradient-tint' }],
      ['complete_task', complete]
    ];
    const [name, args] = calls[turn++]!;
    return streamed(event({ tool_calls: [{ index: 0, id: `c${turn}`, function: { name, arguments: JSON.stringify(args) } }] }, 'tool_calls'));
  }) as typeof fetch;
  const provider = new CompatibleProvider(directory, fetcher);
  await provider.configure({ baseUrl: 'http://localhost:11434/v1', model: 'local', vision: false });
  const result = await provider.run({ id: 'authoring', provider: 'compatible', mode: 'autonomous', access: 'project', projectId: 'proof', projectName: 'Proof', projectJSON: '{}', prompt: 'Create a gradient tint effect', images: [], attachments: [] } as any, () => {}, undefined, {
    extensionsDir: path.join(directory, 'extensions'), apiPackFiles: async () => [{ name: 'api.ts', text: 'export interface PowermoveAPI {}' }],
    onWorkspace: value => { stage = value; }
  });
  expect(result.ok, JSON.stringify(result)).toBe(true);
  if (!result.ok) return;
  expect(result.extensions).toEqual(complete.extensions);
  expect(await readFile(path.join(directory, 'extensions/gradient-tint/index.ts'), 'utf8')).toBe(source);
  expect(JSON.parse(result.text).artifacts).toEqual([]);
  expect(requests[0].messages[0].content).not.toContain('require Claude or ChatGPT');
  expect(requests[1].messages.at(-1).content).toContain('PowermoveAPI');
  expect(requests[4].messages.at(-1).content).toContain('bundlePath');
  await expect(access(stage)).rejects.toThrow();
  await restoreExtensionChangeSet({ liveDirectory: path.join(directory, 'extensions'), historyRoot: path.join(directory, 'Agent Change History/proof'), changeSetId: result.extensionChangeSetId! });
  await expect(access(path.join(directory, 'extensions/gradient-tint'))).rejects.toThrow();
});

it('requires fresh Computer approval before calling an API or preparing a workspace', async () => {
  const fetcher = vi.fn();
  const provider = new CompatibleProvider('/unused', fetcher);
  const result = await provider.run({ id: 'computer', mode: 'autonomous', access: 'computer', consentToken: null } as any, () => {});
  expect(result).toMatchObject({ ok: false, error: 'Computer access requires fresh approval for this run.' });
  expect(fetcher).not.toHaveBeenCalled();
});

it('keeps cancelled extension work in its isolated stage for the next run', async () => {
  const directory = await mkdtemp(path.join(os.tmpdir(), 'pm-api-cancel-')); directories.push(directory);
  let stage = '';
  const fetcher = vi.fn(async (_url: any, options: any) => {
    if (options.method === 'GET') return new Response(null, { status: 404 });
    const body = JSON.parse(options.body);
    if (!body.stream) return Response.json({ choices: [{ message: { content: 'OK' } }] });
    if (body.messages.length === 2) return streamed(event({ tool_calls: [{ index: 0, id: 'write', function: {
      name: 'write_file', arguments: JSON.stringify({ path: `${stage}/partial/index.ts`, text: 'export function activate() {}' })
    } }] }, 'tool_calls'));
    provider.cancel('cancel-run');
    throw new Error('Cancelled');
  }) as typeof fetch;
  const provider = new CompatibleProvider(directory, fetcher);
  await provider.configure({ baseUrl: 'http://localhost:11434/v1', model: 'local', vision: false });
  const req = { id: 'cancel-run', threadId: 'thread', provider: 'compatible', mode: 'autonomous', access: 'project', projectId: 'proof', projectName: 'Proof', projectJSON: '{}', prompt: 'Build an effect', images: [], attachments: [] } as const;
  const extensionsDir = path.join(directory, 'extensions');
  const result = await provider.run(req as any, () => {}, undefined, { extensionsDir, apiPackFiles: async () => [], onWorkspace: value => { stage = value; } });
  expect(result).toMatchObject({ ok: false, cancelled: true });
  expect(await readFile(`${stage}/partial/index.ts`, 'utf8')).toContain('activate');
  await expect(access(path.join(extensionsDir, 'partial'))).rejects.toThrow();
  const resumed = await prepareAgentWorkspace(req as any, directory, 'project', agentResultSchema(), { extensionsDir, apiPackFiles: [] });
  expect(resumed.stagingDirectory).toBe(stage);
});
it('discovers provider models and sends the selected model and reasoning on every tool turn', async () => {
  const directory = await mkdtemp(path.join(os.tmpdir(), 'pm-model-picker-')); directories.push(directory);
  const requests: any[] = [];
  const fetcher = vi.fn(async (_url: any, options: any) => {
    if (options.method === 'GET') return Response.json({ data: [{ id: 'gpt-6-astra' }, { id: 'gpt-5.6-sol' }, { id: 'gpt-6-astra' }] });
    const body = JSON.parse(options.body); requests.push(body);
    if (!body.stream) return Response.json({ choices: [{ message: { content: 'OK' } }] });
    if (body.messages.length === 2) return streamed(event({ tool_calls: [{ index: 0, id: 'c1', function: { name: 'get_project_state', arguments: '{}' } }] }, 'tool_calls'));
    return streamed(event({ content: 'Ready.' }, 'stop'));
  }) as typeof fetch;
  const provider = new CompatibleProvider(directory, fetcher);
  const config = await provider.configure({ baseUrl: 'https://example.com/v1', model: 'gpt-6-astra', vision: false });
  expect(config.models).toEqual(['gpt-6-astra', 'gpt-5.6-sol']);
  const result = await provider.run({ id: 'r', mode: 'autonomous', access: 'editor', model: 'gpt-5.6-sol', reasoningEffort: 'max', prompt: 'Hello', images: [], attachments: [] } as any, () => {},
    async () => ({ runId: 'r', callId: 'c1', ok: true, content: [] }));
  expect(result.ok).toBe(true);
  expect(requests.filter(body => body.stream)).toHaveLength(2);
  expect(requests.filter(body => body.stream).every(body => body.model === 'gpt-5.6-sol' && body.reasoning_effort === 'max')).toBe(true);
});
function streamed(text: string) {
  const bytes = new TextEncoder().encode(text);
  return new Response(new ReadableStream({ start(controller) { for (let i = 0; i < bytes.length; i += 3) controller.enqueue(bytes.slice(i, i + 3)); controller.close(); } }));
}
it('decodes split Unicode and tool arguments and rejects truncated streams', async () => {
  const deltas: string[] = [];
  const response = await readCompletion(streamed(event({ content: 'Hi 🟢' }) + event({ tool_calls: [{ index: 0, id: 'c1', function: { name: 'get_project_state', arguments: '{' } }] }) + event({ tool_calls: [{ index: 0, function: { arguments: '}' } }] }, 'tool_calls')), new AbortController().signal, value => deltas.push(value));
  expect(deltas.join('')).toBe('Hi 🟢'); expect(response.tool_calls?.[0].function.arguments).toBe('{}');
  await expect(readCompletion(streamed(event({ content: 'partial' })), new AbortController().signal, () => {})).rejects.toThrow('before the model finished');
});
it('reassembles JSON split across compatible SSE data records', async () => {
  const payload = JSON.stringify({ choices: [{ delta: { content: 'Recovered response.' }, finish_reason: 'stop' }] });
  for (const splitAt of [payload.indexOf('response'), payload.length - 2]) {
    const response = await readCompletion(streamed(
      `data: ${payload.slice(0, splitAt)}\n\ndata: ${payload.slice(splitAt)}\n\n`
    ), new AbortController().signal, () => {});
    expect(response.content).toBe('Recovered response.');
  }
});
it('does not expose JSON parser exceptions for malformed provider events', async () => {
  await expect(readCompletion(streamed('data: {"choices": nope}\n\n'), new AbortController().signal, () => {}))
    .rejects.toThrow('invalid response stream');
  await expect(readCompletion(streamed('data: null\n\n'), new AbortController().signal, () => {}))
    .rejects.toThrow('invalid response stream');
});
it('keeps keys out of status, does not forward saved keys to another provider, and uses native tools', async () => {
  const directory = await mkdtemp(path.join(os.tmpdir(), 'pm-provider-')); directories.push(directory);
  const requests: any[] = [];
  const fetcher = vi.fn(async (_url: any, options: any) => {
    if (options.method === 'GET') return new Response(null, { status: 404 });
    const body = JSON.parse(options.body); requests.push({ body, headers: options.headers });
    if (!body.stream) return Response.json({ choices: [{ message: { content: 'OK' } }] });
    if (body.messages.length === 2) return streamed(event({ tool_calls: [{ index: 0, id: 'c1', function: { name: 'get_panel_state', arguments: '{\"panelId\":\"inspector\"}' } }] }, 'tool_calls'));
    return streamed(event({ content: 'Ready  ' }) + event({ content: 'to\nedit.\u0000' }, 'stop'));
  }) as unknown as typeof fetch;
  const provider = new CompatibleProvider(directory, fetcher);
  const connected = await provider.configure({ baseUrl: 'https://example.com/v1', model: 'model-a', apiKey: 'private-key', vision: false });
  expect(JSON.stringify(connected)).not.toContain('private-key');
  expect(await readFile(path.join(directory, 'agent-provider.json'), 'utf8')).not.toContain('private-key');
  await provider.configure({ baseUrl: 'http://localhost:11434/v1', model: 'local', vision: false });
  expect(requests[1].headers.Authorization).toBeUndefined();
  const call = vi.fn(async () => ({ runId: 'r', callId: 'c1', ok: true, content: [{ type: 'text' as const, text: 'Project is open' }] }));
  const trace: any[] = [];
  const result = await provider.run({ id: 'r', mode: 'autonomous', access: 'editor', prompt: 'Hello', images: [], attachments: [] } as any, step => trace.push(step), call);
  expect(call).toHaveBeenCalledWith('get_panel_state', { panelId: 'inspector' });
  expect(result.ok && JSON.parse(result.text).summary).toBe('Ready  to\nedit.\u0000');
  expect(trace).toEqual([
    { kind: 'tool-start', itemId: 'c1', toolName: 'get_panel_state', label: 'Get panel state', detail: 'inspector' },
    { kind: 'tool-end', itemId: 'c1', isError: false, output: 'Project is open' },
    { kind: 'answer', text: 'Ready  ' },
    { kind: 'answer', text: 'to\nedit.' }
  ]);
});
it('exposes only live inspection tools to compatible editor runs and forwards captured images', async () => {
  const directory = await mkdtemp(path.join(os.tmpdir(), 'pm-provider-editor-')); directories.push(directory);
  const requests: any[] = [];
  const fetcher = vi.fn(async (_url: any, options: any) => {
    if (options.method === 'GET') return new Response(null, { status: 404 });
    const body = JSON.parse(options.body); requests.push(body);
    if (!body.stream) return Response.json({ choices: [{ message: { content: 'OK' } }] });
    if (body.messages.length === 2) return streamed(event({ tool_calls: [
      { index: 0, id: 'capture-1', function: { name: 'capture_panel', arguments: '{"panelId":"inspector"}' } },
      { index: 1, id: 'state-1', function: { name: 'get_workspace_state', arguments: '{}' } }
    ] }, 'tool_calls'));
    return streamed(event({ content: '{"kind":"panels"}' }, 'stop'));
  }) as unknown as typeof fetch;
  const provider = new CompatibleProvider(directory, fetcher);
  await provider.configure({ baseUrl: 'http://localhost:11434/v1', model: 'local', vision: true });
  const call = vi.fn(async () => ({
    runId: 'editor-run', callId: 'capture-1', ok: true,
    content: [{ type: 'image' as const, mimeType: 'image/png' as const, data: new Uint8Array([137, 80, 78, 71]) }]
  }));
  const result = await provider.run({
    id: 'editor-run', mode: 'editor', access: 'editor', prompt: 'Inspect it', schema: { type: 'object' },
    images: [], attachments: []
  } as any, () => {}, call);

  const firstTurn = requests.find((body) => body.stream && body.messages.length === 2);
  const toolNames = firstTurn.tools.map((tool: any) => tool.function.name);
  expect(toolNames).toContain('capture_panel');
  expect(toolNames).not.toContain('apply_commands');
  expect(toolNames).not.toContain('computer_use_panel');
  expect(requests.at(-1).messages.at(-1).content[0].image_url.url).toBe('data:image/png;base64,iVBORw==');
  expect(requests.at(-1).messages.slice(-3).map((message: any) => message.role)).toEqual(['tool', 'tool', 'user']);
  expect(call).toHaveBeenCalledWith('capture_panel', { panelId: 'inspector' });
  expect(result).toMatchObject({ ok: true, text: '{"kind":"panels"}', access: 'editor' });
});
it('allows local HTTP and rejects credentials, insecure remote hosts and URL fragments', () => {
  expect(providerUrl('http://localhost:11434/v1/')).toBe('http://localhost:11434/v1');
  for (const url of ['http://example.com/v1', 'https://key@example.com', 'https://example.com/#key', 'file:///tmp/model']) expect(() => providerUrl(url)).toThrow();
});

it('accepts Sub2API server URLs and keys through the existing backend connection', async () => {
  const { createServer } = await import('node:http');
  const requests: { url?: string; authorization?: string; body: any }[] = [];
  const server = createServer(async (req, res) => {
    if (req.method === 'GET') { res.writeHead(404).end(); return; }
    let body = '';
    for await (const chunk of req) body += chunk;
    const payload = JSON.parse(body);
    requests.push({ url: req.url, authorization: req.headers.authorization, body: payload });
    if (req.url !== '/v1/chat/completions' || req.headers.authorization !== 'Bearer sk-sub2api-test') {
      res.writeHead(401).end(); return;
    }
    if (!payload.stream) {
      res.writeHead(200, { 'Content-Type': 'application/json' });
      res.end(JSON.stringify({ choices: [{ message: { content: 'OK' } }] })); return;
    }
    res.writeHead(200, { 'Content-Type': 'text/event-stream' });
    res.end(event({ content: 'Connected through the gateway.' }, 'stop') + 'data: [DONE]\n\n');
  });
  await new Promise<void>(resolve => server.listen(0, '127.0.0.1', resolve));
  try {
    const address = server.address() as import('node:net').AddressInfo;
    const baseUrl = `http://127.0.0.1:${address.port}`;
    const directory = await mkdtemp(path.join(os.tmpdir(), 'pm-sub2api-')); directories.push(directory);
    const provider = new CompatibleProvider(directory);
    const config = await provider.configure({ baseUrl, apiKey: 'sk-sub2api-test', model: 'claude-group-alias', vision: false });
    expect(config).toMatchObject({ baseUrl: baseUrl + '/v1', hasKey: true });
    const reopened = new CompatibleProvider(directory);
    // Re-entering the equivalent full endpoint must retain the encrypted key.
    await reopened.configure({ baseUrl: baseUrl + '/v1/chat/completions', model: 'claude-group-alias', vision: false });
    const result = await reopened.run({ id: 'sub2api-run', mode: 'autonomous', access: 'editor', prompt: 'Hello', images: [], attachments: [] } as any, () => {});
    expect(result.ok && JSON.parse(result.text).summary).toBe('Connected through the gateway.');
    expect(requests).toHaveLength(3);
    expect(requests.every(request => request.authorization === 'Bearer sk-sub2api-test' && request.url === '/v1/chat/completions')).toBe(true);
    expect(requests[2]!.body.model).toBe('claude-group-alias');
    expect(JSON.stringify(await reopened.status())).not.toContain('sk-sub2api-test');
    expect(await readFile(path.join(directory, 'agent-provider.json'), 'utf8')).not.toContain('sk-sub2api-test');
    await expect(reopened.configure({ baseUrl, apiKey: 'invalid-key', model: 'claude-group-alias', vision: false })).rejects.toThrow('rejected the API key');
    expect(await reopened.status()).toEqual(config);
  } finally {
    server.closeAllConnections();
    await new Promise<void>(resolve => server.close(() => resolve()));
  }
});

it('normalizes gateway origins and complete endpoints without dropping deployment prefixes', () => {
  expect(providerUrl('https://gateway.example')).toBe('https://gateway.example/v1');
  expect(providerUrl('https://gateway.example/v1/')).toBe('https://gateway.example/v1');
  expect(providerUrl('https://gateway.example/team/v1/chat/completions')).toBe('https://gateway.example/team/v1');
  expect(providerUrl('https://gateway.example/team/v1')).toBe('https://gateway.example/team/v1');
});
