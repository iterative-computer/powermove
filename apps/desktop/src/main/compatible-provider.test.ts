import { afterEach, expect, it, vi } from 'vitest';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import path from 'node:path';
import os from 'node:os';
vi.mock('electron', () => ({ safeStorage: { isEncryptionAvailable: () => true, encryptString: (value: string) => Buffer.from('encrypted:' + value), decryptString: (value: Buffer) => value.toString().slice(10) } }));
import { CompatibleProvider, readCompletion } from './compatible-provider';
import { providerUrl } from '../shared/compatible-provider';
const directories: string[] = [];
afterEach(async () => { for (const dir of directories.splice(0)) await rm(dir, { recursive: true, force: true }); });
const event = (delta: any, finish_reason: any = null) => `data: ${JSON.stringify({ choices: [{ delta, finish_reason }] })}\r\n\r\n`;
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
it('keeps keys out of status, does not forward saved keys to another provider, and uses native tools', async () => {
  const directory = await mkdtemp(path.join(os.tmpdir(), 'pm-provider-')); directories.push(directory);
  const requests: any[] = [];
  const fetcher = vi.fn(async (_url: any, options: any) => {
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
    const body = JSON.parse(options.body); requests.push(body);
    if (!body.stream) return Response.json({ choices: [{ message: { content: 'OK' } }] });
    if (body.messages.length === 2) return streamed(event({ tool_calls: [{ index: 0, id: 'capture-1', function: { name: 'capture_panel', arguments: '{"panelId":"inspector"}' } }] }, 'tool_calls'));
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
  expect(call).toHaveBeenCalledWith('capture_panel', { panelId: 'inspector' });
  expect(result).toMatchObject({ ok: true, text: '{"kind":"panels"}', access: 'editor' });
});
it('allows local HTTP and rejects credentials, insecure remote hosts and URL fragments', () => {
  expect(providerUrl('http://localhost:11434/v1/')).toBe('http://localhost:11434/v1');
  for (const url of ['http://example.com/v1', 'https://key@example.com', 'https://example.com/#key', 'file:///tmp/model']) expect(() => providerUrl(url)).toThrow();
});
