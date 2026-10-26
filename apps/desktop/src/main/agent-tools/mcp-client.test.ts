import { mkdtemp, rm } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { afterEach, expect, it, vi } from 'vitest';
import { connectMcpHost } from './mcp-client.mjs';
import { startExternalMcp } from './external-server';
import type { ExternalAgentSession } from './external-session';

const cleanup: Array<() => Promise<unknown>> = [];
afterEach(async () => { for (const close of cleanup.splice(0).reverse()) await close(); });

async function profile() {
  const root = await mkdtemp(path.join(os.tmpdir(), 'powermove-mcp-reconnect-'));
  cleanup.push(() => rm(root, { recursive: true, force: true }));
  return root;
}

async function host(root: string, call = vi.fn(async (_name: string, _args: Record<string, unknown>): Promise<unknown> => ({ connected: true }))) {
  const createSession = vi.fn(() => ({
    tools: () => [{ name: 'start_session' }], call, close: async () => {}
  }) as unknown as ExternalAgentSession);
  const instance = await startExternalMcp({ userData: root, createSession });
  cleanup.push(() => instance.close());
  return { ...instance, call, createSession };
}

it('reconnects through updated host discovery without replacing the MCP client', async () => {
  const root = await profile(), first = await host(root);
  const client = await connectMcpHost(root);
  cleanup.push(() => client.close());
  expect(await client.listTools()).toEqual([{ name: 'start_session' }]);
  await first.close();
  // Let the client observe EOF before the next request.
  await new Promise(resolve => setImmediate(resolve));
  await new Promise(resolve => setImmediate(resolve));
  const second = await host(root);
  await expect(client.callTool('start_session', { prompt: 'Inspect recovered project' })).resolves.toMatchObject({ isError: false });
  expect(second.call).toHaveBeenCalledExactlyOnceWith('start_session', { prompt: 'Inspect recovered project' });
  await client.close();
  await expect(client.listTools()).rejects.toThrow('connection is closed');
});

it('reports interrupted edits once and never replays them on the replacement host', async () => {
  const root = await profile();
  const first = await host(root, vi.fn(() => new Promise(() => {})));
  const client = await connectMcpHost(root);
  cleanup.push(() => client.close());
  const interrupted = client.callTool('apply_commands', { commands: [{ op: 'delete_layer', layerId: 'layer-one' }] });
  const rejection = expect(interrupted).rejects.toThrow('action outcome is unknown');
  await vi.waitFor(() => expect(first.call).toHaveBeenCalledOnce());
  await first.close();
  await rejection;
  const second = await host(root);
  await client.listTools();
  expect(second.call).not.toHaveBeenCalled();
  await client.callTool('start_session', { prompt: 'Inspect before retrying' });
  expect(second.call).toHaveBeenCalledOnce();
});

it('can reconnect after an earlier discovery retry failed while the app was stopped', async () => {
  const root = await profile(), first = await host(root);
  const client = await connectMcpHost(root);
  cleanup.push(() => client.close());
  await first.close();
  await new Promise(resolve => setImmediate(resolve));
  await new Promise(resolve => setImmediate(resolve));
  await expect(client.listTools()).rejects.toThrow('No Powermove MCP host');
  await host(root);
  await expect(client.listTools()).resolves.toEqual([{ name: 'start_session' }]);
});

it('reports when the app ends the connection, but not when the client closes it', async () => {
  const root = await profile(), first = await host(root);
  const onHostClosed = vi.fn();
  const client = await connectMcpHost(root, { onHostClosed });
  cleanup.push(() => client.close());
  await client.listTools();
  await first.close();
  await vi.waitFor(() => expect(onHostClosed).toHaveBeenCalledOnce());

  const second = await host(root);
  const quiet = vi.fn();
  const other = await connectMcpHost(root, { onHostClosed: quiet });
  await other.close();
  await second.close();
  await new Promise(resolve => setImmediate(resolve));
  expect(quiet).not.toHaveBeenCalled();
});
