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
    if (body.messages.length === 2) return streamed(event({ tool_calls: [{ index: 0, id: 'c1', function: { name: 'get_project_state', arguments: '{}' } }] }, 'tool_calls'));
    return streamed(event({ content: 'Ready to edit.' }, 'stop'));
  }) as unknown as typeof fetch;
  const provider = new CompatibleProvider(directory, fetcher);
  const connected = await provider.configure({ baseUrl: 'https://example.com/v1', model: 'model-a', apiKey: 'private-key', vision: false });
  expect(JSON.stringify(connected)).not.toContain('private-key');
  expect(await readFile(path.join(directory, 'agent-provider.json'), 'utf8')).not.toContain('private-key');
  await provider.configure({ baseUrl: 'http://localhost:11434/v1', model: 'local', vision: false });
  expect(requests[1].headers.Authorization).toBeUndefined();
  const call = vi.fn(async () => ({ runId: 'r', callId: 'c1', ok: true, content: [{ type: 'text' as const, text: 'Project is open' }] }));
  const result = await provider.run({ id: 'r', mode: 'autonomous', access: 'editor', prompt: 'Hello', images: [], attachments: [] } as any, () => {}, call);
  expect(call).toHaveBeenCalledWith('get_project_state', {});
  expect(result.ok && JSON.parse(result.text).summary).toBe('Ready to edit.');
});
it('allows local HTTP and rejects credentials, insecure remote hosts and URL fragments', () => {
  expect(providerUrl('http://localhost:11434/v1/')).toBe('http://localhost:11434/v1');
  for (const url of ['http://example.com/v1', 'https://key@example.com', 'https://example.com/#key', 'file:///tmp/model']) expect(() => providerUrl(url)).toThrow();
});
