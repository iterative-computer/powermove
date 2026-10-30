import { afterEach, describe, expect, it, vi } from 'vitest';
import { mkdtemp, readFile, rm, access, writeFile } from 'node:fs/promises';
import path from 'node:path';
import os from 'node:os';
vi.mock('electron', () => ({ safeStorage: { isEncryptionAvailable: () => true, encryptString: (value: string) => Buffer.from('encrypted:' + value), decryptString: (value: Buffer) => value.toString().slice(10) } }));
import { CompatibleProvider, readCompletion } from './compatible-provider';
import { presetFor, providerUrl } from '../shared/compatible-provider';
import { restoreExtensionChangeSet } from './codex/change-history';
import { prepareAgentWorkspace } from './codex/workspace';
import { AGENT_SHELL_NETWORK_INSTRUCTIONS, agentResultSchema } from './codex/instructions';
const directories: string[] = [];
afterEach(async () => { for (const dir of directories.splice(0)) await rm(dir, { recursive: true, force: true }); });
const event = (delta: any, finish_reason: any = null) => `data: ${JSON.stringify({ choices: [{ delta, finish_reason }] })}\r\n\r\n`;
it('automatically offers and invokes complete video delivery after connecting an API provider', async () => {
  const directory = await mkdtemp(path.join(os.tmpdir(), 'pm-api-delivery-')); directories.push(directory);
  const requests: any[] = [], calls = ['export_video', 'save_project', 'complete_task'];
  let turn = 0;
  const fetcher = vi.fn(async (_url: any, options: any) => {
    if (options.method === 'GET') return new Response(null, { status: 404 });
    const body = JSON.parse(options.body);
    if (!body.stream) return Response.json({ choices: [{ message: { content: 'OK' } }] });
    requests.push(body);
    const name = calls[turn++]!;
    const args = name === 'complete_task' ? { summary: 'Video delivered.', commands: [], artifacts: [], extensions: [], notes: [], externalActions: [] } : {};
    return streamed(event({ tool_calls: [{ index: 0, id: `delivery-${turn}`, function: { name, arguments: JSON.stringify(args) } }] }, 'tool_calls'));
  }) as typeof fetch;
  const provider = new CompatibleProvider(directory, fetcher);
  await provider.configure({ baseUrl: 'http://localhost:11434/v1', model: 'local', vision: false });
  const call = vi.fn(async (name: string) => ({ runId: 'delivery', callId: name, ok: true, content: [{ type: 'text' as const, text: JSON.stringify({ path: `/tmp/${name}` }) }] }));
  const result = await provider.run({ id: 'delivery', provider: 'compatible', mode: 'autonomous', access: 'project', projectId: 'proof', projectName: 'Proof', projectJSON: '{}', prompt: 'Finish the video', images: [], attachments: [] } as any, () => {}, call, {
    extensionsDir: path.join(directory, 'extensions'), apiPackFiles: async () => [{ name: 'api.ts', text: 'export interface PowermoveAPI {}' }]
  });
  expect(result.ok, JSON.stringify(result)).toBe(true);
  const names = requests[0].tools.map((tool: any) => tool.function.name);
  expect(names).toEqual(expect.arrayContaining(['import_media', 'edit_video', 'render_frames', 'save_project', 'export_video']));
  expect(call.mock.calls.map(([name]) => name)).toEqual(['export_video', 'save_project']);
});
it('returns a valid, explicit recovery response rather than cutting off large project JSON', async () => {
  const directory = await mkdtemp(path.join(os.tmpdir(), 'pm-api-pages-')); directories.push(directory);
  let turn = 0;
  let recovered: any;
  const fetcher = vi.fn(async (_url: any, options: any) => {
    if (options.method === 'GET') return new Response(null, { status: 404 });
    const body = JSON.parse(options.body);
    if (!body.stream) return Response.json({ choices: [{ message: { content: 'OK' } }] });
    if (turn++ === 0) return streamed(event({ tool_calls: [{ index: 0, id: 'read', function: { name: 'get_project_state', arguments: '{}' } }] }, 'tool_calls'));
    recovered = JSON.parse(body.messages.at(-1).content);
    return streamed(event({ content: JSON.stringify({ operation: 'noop', message: 'Inspected the project.' }) }, 'stop'));
  }) as typeof fetch;
  const provider = new CompatibleProvider(directory, fetcher);
  await provider.configure({ baseUrl: 'http://localhost:11434/v1', model: 'local', vision: false });
  const result = await provider.run({ id: 'pages', provider: 'compatible', mode: 'editor', access: 'editor', prompt: 'Inspect the project', schema: {}, images: [], attachments: [] } as any, () => {}, async () => ({
    runId: 'pages', callId: 'read', ok: true, content: [{ type: 'text', text: JSON.stringify({ layers: [{ id: 'title', content: 'x'.repeat(160_000) }], pagination: { nextLayerOffset: 12 } }) }],
  }));
  expect(result.ok, JSON.stringify(result)).toBe(true);
  expect(recovered.ok).toBe(true);
  expect(recovered.outputTruncated).toBe(true);
  expect(recovered.content[0].text).toContain('indexOnly');
  expect(recovered.content[0].text).toContain('layerId');
});

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
  // Project shells reach any host.
  expect(requests[0].messages[0].content).toContain(AGENT_SHELL_NETWORK_INSTRUCTIONS);
  expect(AGENT_SHELL_NETWORK_INSTRUCTIONS).toBe('Shell commands have full internet access.');
  expect(requests[0].messages[0].content).not.toMatch(/download only|allowlist/);
  expect(requests[1].messages.at(-1).content).toContain('PowermoveAPI');
  // The compile check reports its result; its bundle stays out of the workspace.
  expect(requests[4].messages.at(-1).content).toContain('\\"hash\\":');
  expect(requests[4].messages.at(-1).content).not.toContain('bundlePath');
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
it.runIf(process.platform === 'darwin')('stops background jobs when a run ends, even when it fails', async () => {
  const directory = await mkdtemp(path.join(os.tmpdir(), 'pm-api-jobs-')); directories.push(directory);
  let stage = '';
  const fetcher = vi.fn(async (_url: any, options: any) => {
    if (options.method === 'GET') return new Response(null, { status: 404 });
    const body = JSON.parse(options.body);
    if (!body.stream) return Response.json({ choices: [{ message: { content: 'OK' } }] });
    if (body.messages.length === 2) return streamed(event({ tool_calls: [{ index: 0, id: 'job', function: {
      name: 'run_command', arguments: JSON.stringify({ command: 'echo $$ > job.pid; exec sleep 30', background: true })
    } }] }, 'tool_calls'));
    await expect.poll(() => readFile(pidFile(), 'utf8').then(text => /^\d+\n$/.test(text), () => false)).toBe(true);
    throw new Error('Provider went away');
  }) as typeof fetch;
  const pidFile = () => path.join(stage, '..', '..', '..', 'job.pid');
  const provider = new CompatibleProvider(directory, fetcher);
  await provider.configure({ baseUrl: 'http://localhost:11434/v1', model: 'local', vision: false });
  const req = { id: 'job-run', provider: 'compatible', mode: 'autonomous', access: 'project', projectId: 'proof', projectName: 'Proof', projectJSON: '{}', prompt: 'Render', images: [], attachments: [] };
  const result = await provider.run(req as any, () => {}, undefined, { extensionsDir: path.join(directory, 'extensions'), apiPackFiles: async () => [], onWorkspace: value => { stage = value; } });
  expect(result).toMatchObject({ ok: false, cancelled: false });
  const pid = Number(await readFile(pidFile(), 'utf8'));
  expect(pid).toBeGreaterThan(1);
  const alive = () => { try { process.kill(pid, 0); return true; } catch { return false; } };
  await expect.poll(alive).toBe(false);
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

describe('connecting the way people with API keys and local models work', () => {
  const ok = () => Response.json({ choices: [{ message: { content: 'OK' } }] });
  const temp = async () => { const directory = await mkdtemp(path.join(os.tmpdir(), 'pm-provider-flow-')); directories.push(directory); return directory; };

  it('accepts a bare local host as people copy it from a server log', () => {
    expect(providerUrl('localhost:11434')).toBe('http://localhost:11434/v1');
    expect(providerUrl('openrouter.ai/api/v1')).toBe('https://openrouter.ai/api/v1');
    expect(presetFor('http://localhost:1234/v1/').id).toBe('lmstudio');
    expect(presetFor('https://example.com/v1').id).toBe('other');
  });

  it('keeps a key per address, so switching to a local model and back never asks again', async () => {
    const auth: Array<string | undefined> = [];
    const fetcher = vi.fn(async (_url: any, options: any) => {
      if (options.method === 'GET') return new Response(null, { status: 404 });
      auth.push(options.headers.Authorization);
      return ok();
    }) as unknown as typeof fetch;
    const provider = new CompatibleProvider(await temp(), fetcher);
    await provider.configure({ baseUrl: 'https://openrouter.ai/api/v1', model: 'a', apiKey: 'sk-or-1', vision: false });
    await provider.configure({ baseUrl: 'https://api.openai.com/v1', model: 'b', apiKey: 'sk-2', vision: false });
    expect(await provider.configure({ baseUrl: 'http://localhost:11434/v1', model: 'c', vision: false })).toMatchObject({ hasKey: false });
    expect(await provider.configure({ baseUrl: 'https://openrouter.ai/api/v1', model: 'a', vision: false })).toMatchObject({ hasKey: true });
    expect(auth).toEqual(['Bearer sk-or-1', 'Bearer sk-2', undefined, 'Bearer sk-or-1']);
    expect((await provider.listModels({ baseUrl: 'https://api.openai.com/v1' })).hasKey).toBe(true);
  });

  it('still uses the single key older versions saved', async () => {
    const directory = await temp();
    await writeFile(path.join(directory, 'agent-provider.json'), JSON.stringify({ baseUrl: 'https://example.com/v1', model: 'm', vision: false, hasKey: true, secret: Buffer.from('encrypted:old-key').toString('base64') }));
    const auth: string[] = [];
    const provider = new CompatibleProvider(directory, vi.fn(async (_url: any, options: any) => {
      if (options.method === 'GET') return new Response(null, { status: 404 });
      auth.push(options.headers.Authorization); return ok();
    }) as unknown as typeof fetch);
    expect(await provider.status()).toMatchObject({ hasKey: true });
    await provider.configure({ baseUrl: 'https://example.com/v1/', model: 'm', vision: false });
    expect(auth).toEqual(['Bearer old-key']);
  });

  it('lists models before connecting, leaving out ones that cannot call tools', async () => {
    const fetcher = vi.fn(async (url: any) => {
      expect(String(url)).toBe('https://openrouter.ai/api/v1/models');
      return Response.json({ data: [
        { id: 'vision-model', architecture: { input_modalities: ['text', 'image'] }, supported_parameters: ['tools'] },
        { id: 'chat-only', supported_parameters: ['temperature'] },
        { id: 'plain' }
      ] });
    }) as unknown as typeof fetch;
    const list = await new CompatibleProvider(await temp(), fetcher).listModels({ baseUrl: 'https://openrouter.ai/api/v1', apiKey: 'sk-or-x' });
    expect(list).toEqual({ models: [{ id: 'vision-model', vision: true }, { id: 'plain' }], hidden: 1, hasKey: false });
  });

  it('reads Ollama capabilities, so embedding and chat-only models are not offered', async () => {
    const fetcher = vi.fn(async (url: any, options: any) => {
      const target = String(url);
      if (target.endsWith('/v1/models')) return Response.json({ data: [{ id: 'qwen3:latest' }, { id: 'nomic-embed-text:latest' }, { id: 'llava:latest' }] });
      if (target.endsWith('/api/version')) return Response.json({ version: '0.12.0' });
      if (target.endsWith('/api/show')) {
        const { model } = JSON.parse(options.body);
        return Response.json({ capabilities: model.startsWith('qwen3') ? ['completion', 'tools'] : model.startsWith('llava') ? ['completion', 'tools', 'vision'] : ['embedding'] });
      }
      return new Response(null, { status: 404 });
    }) as unknown as typeof fetch;
    const list = await new CompatibleProvider(await temp(), fetcher).listModels({ baseUrl: 'http://localhost:11434/v1' });
    expect(list.models).toEqual([{ id: 'llava:latest', vision: true }, { id: 'qwen3:latest', vision: false }]);
    expect(list.hidden).toBe(1);
  });

  it('says what to do when a local server is not running or a cloud key is wrong', async () => {
    const refused = vi.fn(async () => { throw new TypeError('fetch failed'); }) as unknown as typeof fetch;
    const provider = new CompatibleProvider(await temp(), refused);
    await expect(provider.listModels({ baseUrl: 'http://localhost:11434/v1' })).rejects.toThrow('Ollama isn’t running. Open the Ollama app');
    await expect(provider.configure({ baseUrl: 'http://localhost:1234/v1', model: 'm', vision: false })).rejects.toThrow('LM Studio isn’t answering at localhost:1234');
    await expect(provider.configure({ baseUrl: 'https://api.example.com/v1', model: 'm', vision: false })).rejects.toThrow('Couldn’t reach api.example.com');
    const rejected = new CompatibleProvider(await temp(), vi.fn(async () => Response.json({ error: { message: 'Incorrect API key provided: sk-ab***yz' } }, { status: 401 })) as unknown as typeof fetch);
    await expect(rejected.listModels({ baseUrl: 'https://api.openai.com/v1', apiKey: 'sk-abcxyz' })).rejects.toThrow('The provider rejected the API key. Check the key in Settings.');
  });

  it('refuses a model that cannot use tools when testing the connection, with the reason', async () => {
    const bodies: any[] = [];
    const fetcher = vi.fn(async (_url: any, options: any) => {
      if (options.method === 'GET') return new Response(null, { status: 404 });
      bodies.push(JSON.parse(options.body));
      return Response.json({ error: { message: 'registry.ollama.ai/library/gemma2:latest does not support tools' } }, { status: 400 });
    }) as unknown as typeof fetch;
    await expect(new CompatibleProvider(await temp(), fetcher).configure({ baseUrl: 'http://localhost:11434/v1', model: 'gemma2', vision: false }))
      .rejects.toThrow('gemma2 can chat but can’t use tools, so it can’t edit your project.');
    expect(bodies[0].tools[0].function.name).toBe('powermove_check');
    const quota = new CompatibleProvider(await temp(), vi.fn(async (_url: any, options: any) => options.method === 'GET' ? new Response(null, { status: 404 })
      : Response.json({ error: { message: 'Insufficient credits.\nAdd more.' } }, { status: 402 })) as unknown as typeof fetch);
    await expect(quota.configure({ baseUrl: 'https://openrouter.ai/api/v1', model: 'x', apiKey: 'k', vision: false })).rejects.toThrow('HTTP 402: Insufficient credits. Add more.');
  });

  it('warns when Ollama runs the model with too little context for the agent', async () => {
    const context = { value: 4096 };
    const fetcher = vi.fn(async (url: any, options: any) => {
      const target = String(url);
      if (target.endsWith('/api/version')) return Response.json({ version: '0.12.0' });
      if (target.endsWith('/api/ps')) return Response.json({ models: [{ name: 'qwen3:latest', model: 'qwen3:latest', context_length: context.value }] });
      if (options.method === 'GET') return new Response(null, { status: 404 });
      return ok();
    }) as unknown as typeof fetch;
    const provider = new CompatibleProvider(await temp(), fetcher);
    const small = await provider.configure({ baseUrl: 'http://localhost:11434/v1', model: 'qwen3', vision: false });
    expect(small.warning).toContain('only 4,096 tokens of context');
    expect(small.warning).toContain('OLLAMA_CONTEXT_LENGTH');
    context.value = 32768;
    expect((await provider.configure({ baseUrl: 'http://localhost:11434/v1', model: 'qwen3', vision: false })).warning).toBeUndefined();
  });

  it('finds local servers that are already running', async () => {
    const fetcher = vi.fn(async (url: any) => String(url) === 'http://localhost:1234/v1/models'
      ? Response.json({ data: [{ id: 'a' }, { id: 'b' }] })
      : Promise.reject(new TypeError('fetch failed'))) as unknown as typeof fetch;
    expect(await new CompatibleProvider(await temp(), fetcher).detectLocal()).toEqual([{ preset: 'lmstudio', baseUrl: 'http://localhost:1234/v1', models: 2 }]);
  });
});
