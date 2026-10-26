import { EventEmitter } from 'node:events';
import { mkdtemp, mkdir, writeFile, readFile, rm } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { afterEach, expect, it, vi } from 'vitest';
import { IPC, type AgentToolRequestEvent } from '../../shared/ipc';

const native = vi.hoisted(() => ({
  beforeRename: null as null | ((from: string, to: string) => Promise<void>),
  beforeReadDirectory: null as null | ((directory: string) => Promise<void>)
}));
vi.mock('node:fs/promises', async importOriginal => {
  const fs = await importOriginal<typeof import('node:fs/promises')>();
  return { ...fs,
    rename: async (from: string, to: string) => { await native.beforeRename?.(from, to); return fs.rename(from, to); },
    readdir: async (directory: string, ...args: any[]) => { await native.beforeReadDirectory?.(directory); return (fs.readdir as any)(directory, ...args); }
  };
});

it('does not publish a disconnected socket request queued behind non-mutating stage preparation', async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), 'powermove-mcp-queued-disconnect-'));
  const live = path.join(root, 'extensions'), id = 'safe-change';
  await mkdir(path.join(live, id), { recursive: true });
  await writeFile(path.join(live, id, 'manifest.json'), JSON.stringify({ id, name: id, version: '1.0.0', apiVersion: 1, entry: 'index.ts', author: 'agent' }));
  await writeFile(path.join(live, id, 'index.ts'), 'export default "before";');
  const ipc = new EventEmitter();
  const owner = Object.assign(new EventEmitter(), { id: 1, isDestroyed: () => false, isCrashed: () => false,
    send: (_channel: string, request: AgentToolRequestEvent) => queueMicrotask(() => ipc.emit(IPC.agentToolResponse, { sender: owner }, {
      runId: request.runId, callId: request.callId, ok: true,
      content: [{ type: 'text', text: JSON.stringify(request.tool === '__mcp_context'
        ? { projectId: 'project-one', projectName: 'One', projectJSON: '{"id":"project-one","revision":2,"layers":[]}', home: false } : {}) }], changed: false, revision: 2
    })) });
  const bridge = new PowermoveAgentToolBridge(ipc as any, { mcpServerPath: '', timeoutMs: 1000 });
  const sessions: ExternalAgentSession[] = [];
  const host = await startExternalMcp({ userData: root, createSession: () => {
    const session = new ExternalAgentSession({ bridge, getOwner: () => owner as any, userData: root,
      extensionsDir: live, apiPackFiles: async () => [{ name: 'EXTENSIONS.md', text: 'API documentation' }], attach: async () => async () => {} });
    sessions.push(session); return session;
  } });
  const client = await connectMcpHost(root);
  const unwrap = (result: any) => { expect(result.isError).toBe(false); return JSON.parse(result.content[0].text); };
  let resume!: () => void;
  const pause = new Promise<void>(resolve => { resume = resolve; });
  let held!: () => void;
  const preparing = new Promise<void>(resolve => { held = resolve; });
  let preparation: Promise<unknown> | undefined, request: Promise<unknown> | undefined;
  let recovered: Awaited<ReturnType<typeof connectMcpHost>> | undefined;
  try {
    const opened = unwrap(await client.callTool('start_session', { prompt: 'Update extension' }));
    await writeFile(path.join(opened.stagingDirectory, id, 'index.ts'), 'export default "after";');
    let heldOnce = false;
    native.beforeReadDirectory = async directory => {
      if (!heldOnce && String(directory) === live) { heldOnce = true; held(); await pause; }
    };
    const historyRoot = path.join(root, 'Agent Change History', 'project-one');
    preparation = history.prepareExtensionStage({ liveDirectory: live, historyRoot, stagingDirectory: path.join(root, 'other-stage'), projectId: 'project-one', runId: 'other-stage' });
    await preparing;
    let queued!: () => void;
    const queuedPublication = new Promise<void>(resolve => { queued = resolve; });
    let outcome: any;
    const publish = history.publishExtensionChanges;
    vi.spyOn(history, 'publishExtensionChanges').mockImplementation((...args) => {
      queued();
      const work = publish(...args);
      void work.then(value => { outcome = { value }; }, error => { outcome = { error }; });
      return work;
    });
    request = client.callTool('publish_extensions', { extensions: [{ id, action: 'updated', summary: 'Update extension' }] }).catch(error => error);
    await queuedPublication;
    await client.close();
    await sessions[0]!.close();
    resume(); await preparation;
    await vi.waitFor(() => expect(outcome).toMatchObject({ error: expect.objectContaining({ message: expect.stringContaining('connection is closed') }) }));
    expect(await readFile(path.join(live, id, 'index.ts'), 'utf8')).toBe('export default "before";');
    recovered = await connectMcpHost(root);
    const next = unwrap(await recovered.callTool('start_session', { prompt: 'Publish after recovery' }));
    await writeFile(path.join(next.stagingDirectory, id, 'index.ts'), 'export default "after";');
    unwrap(await recovered.callTool('publish_extensions', { extensions: [{ id, action: 'updated', summary: 'Recovered update' }] }));
    expect(await readFile(path.join(live, id, 'index.ts'), 'utf8')).toBe('export default "after";');
  } finally {
    resume(); await preparation; await request;
    await client.close(); await recovered?.close(); await Promise.all(sessions.map(session => session.close()));
    await host.close(); await bridge.shutdown(); await rm(root, { recursive: true, force: true });
  }
});
import { PowermoveAgentToolBridge } from './bridge';
import { ExternalAgentSession } from './external-session';
import { startExternalMcp } from './external-server';
import { connectMcpHost } from './mcp-client.mjs';
import * as history from '../codex/change-history';

afterEach(() => { native.beforeRename = null; native.beforeReadDirectory = null; vi.restoreAllMocks(); });

it.each(['first-change', 'second-change'])('serializes concurrent %s publication, rejecting only overlapping edits and keeping history restorable', async secondId => {
  const root = await mkdtemp(path.join(os.tmpdir(), 'powermove-mcp-concurrent-publish-'));
  const live = path.join(root, 'extensions');
  for (const id of ['first-change', 'second-change']) {
    await mkdir(path.join(live, id), { recursive: true });
    await writeFile(path.join(live, id, 'manifest.json'), JSON.stringify({ id, name: id, version: '1.0.0', apiVersion: 1, entry: 'index.ts', author: 'agent' }));
    await writeFile(path.join(live, id, 'index.ts'), 'export default "before";');
  }
  const ipc = new EventEmitter();
  const owner = Object.assign(new EventEmitter(), { id: 1, isDestroyed: () => false, isCrashed: () => false,
    send: (_channel: string, request: AgentToolRequestEvent) => queueMicrotask(() => ipc.emit(IPC.agentToolResponse, { sender: owner }, {
      runId: request.runId, callId: request.callId, ok: true,
      content: [{ type: 'text', text: JSON.stringify(request.tool === '__mcp_context'
        ? { projectId: 'project-one', projectName: 'One', projectJSON: '{"id":"project-one","revision":2,"layers":[]}', home: false } : {}) }], changed: false, revision: 2
    })) });
  const bridge = new PowermoveAgentToolBridge(ipc as any, { mcpServerPath: '', timeoutMs: 1000 });
  const sessions: ExternalAgentSession[] = [];
  const host = await startExternalMcp({ userData: root, createSession: () => {
    const session = new ExternalAgentSession({ bridge, getOwner: () => owner as any, userData: root,
      extensionsDir: live, apiPackFiles: async () => [{ name: 'EXTENSIONS.md', text: 'API documentation' }], attach: async () => async () => {} });
    sessions.push(session); return session;
  } });
  const first = await connectMcpHost(root), second = await connectMcpHost(root);
  const unwrap = (result: any) => { if (result.failure) throw result.failure; expect(result.isError).toBe(false); return JSON.parse(result.content[0].text); };
  let resume!: () => void;
  const pause = new Promise<void>(resolve => { resume = resolve; });
  let ready!: () => void;
  const firstAtSwap = new Promise<void>(resolve => { ready = resolve; });
  let held = false;
  let firstResult: Promise<any> | undefined;
  let secondResult: Promise<any> | undefined;
  try {
    const one = unwrap(await first.callTool('start_session', { prompt: 'Update first extension' }));
    const two = unwrap(await second.callTool('start_session', { prompt: 'Update second extension' }));
    expect(sessions).toHaveLength(2);
    expect(one.stagingDirectory).not.toBe(two.stagingDirectory);
    await writeFile(path.join(one.stagingDirectory, 'first-change', 'index.ts'), 'export default "first after";');
    await writeFile(path.join(two.stagingDirectory, secondId, 'index.ts'), 'export default "second after";');
    native.beforeRename = async (from, to) => {
      if (!held && String(from) === path.join(live, 'first-change') && String(to).endsWith(path.join('before', 'first-change'))) {
        held = true; ready(); await pause;
      }
    };
    let secondStarted!: () => void;
    const secondAtPublish = new Promise<void>(resolve => { secondStarted = resolve; });
    let publishing = 0;
    const publish = history.publishExtensionChanges;
    vi.spyOn(history, 'publishExtensionChanges').mockImplementation((stage, changes, assertCurrent) => {
      if (++publishing === 2) secondStarted();
      return publish(stage, changes, assertCurrent);
    });
    firstResult = first.callTool('publish_extensions', { extensions: [{ id: 'first-change', action: 'updated', summary: 'First update' }] }).catch(failure => ({ failure }));
    await firstAtSwap;
    secondResult = second.callTool('publish_extensions', { extensions: [{ id: secondId, action: 'updated', summary: 'Second update' }] }).catch(failure => ({ failure }));
    await secondAtPublish;
    // Both requests have finished real snapshot validation. Keep the first
    // transaction held briefly so an unprotected second commit can finish.
    await new Promise(resolve => setTimeout(resolve, 100));
    expect(await readFile(path.join(live, secondId, 'index.ts'), 'utf8')).toBe('export default "before";');
    resume();
    const last = unwrap(await firstResult);
    const secondOutcome = await secondResult;
    const historyRoot = path.join(root, 'Agent Change History', 'project-one');
    if (secondId === 'second-change') {
      const secondPublished = unwrap(secondOutcome);
      expect(await readFile(path.join(live, secondId, 'index.ts'), 'utf8')).toBe('export default "second after";');
      await expect(history.restoreExtensionChangeSet({ liveDirectory: live, historyRoot, changeSetId: last.changeSetId }))
        .rejects.toThrow('Newer extension changes exist');
      await history.restoreExtensionChangeSet({ liveDirectory: live, historyRoot, changeSetId: secondPublished.changeSetId });
      await history.restoreExtensionChangeSet({ liveDirectory: live, historyRoot, changeSetId: last.changeSetId });
      for (const id of ['first-change', 'second-change']) expect(await readFile(path.join(live, id, 'index.ts'), 'utf8')).toBe('export default "before";');
      return;
    }
    expect(secondOutcome).toMatchObject({ failure: expect.objectContaining({ message: expect.stringContaining('Newer changes exist in these extensions: first-change') }) });
    expect(await readFile(path.join(live, 'first-change', 'index.ts'), 'utf8')).toBe('export default "first after";');
    expect(await readFile(path.join(live, 'second-change', 'index.ts'), 'utf8')).toBe('export default "before";');
    expect(await readFile(path.join(two.stagingDirectory, secondId, 'index.ts'), 'utf8')).toBe('export default "second after";');
    await expect(history.restoreExtensionChangeSet({ liveDirectory: live, historyRoot, changeSetId: last.changeSetId }))
      .resolves.toMatchObject({ id: last.changeSetId });
    // Rejection must release the queue and preserve the staged repair. Once
    // the live tree is back at its baseline, retrying can publish and undo.
    const retried = unwrap(await second.callTool('publish_extensions', { extensions: [{ id: secondId, action: 'updated', summary: 'Second update' }] }));
    expect(await readFile(path.join(live, secondId, 'index.ts'), 'utf8')).toBe('export default "second after";');
    await expect(history.restoreExtensionChangeSet({ liveDirectory: live, historyRoot, changeSetId: retried.changeSetId }))
      .resolves.toMatchObject({ id: retried.changeSetId });
  } finally {
    resume(); await firstResult; await secondResult;
    await first.close(); await second.close(); await Promise.all(sessions.map(session => session.close())); await host.close(); await bridge.shutdown();
    await rm(root, { recursive: true, force: true });
  }
});
