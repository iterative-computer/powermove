import { describe, expect, it, vi } from 'vitest';

import { IPC } from '../shared/ipc';
import { WEB } from '../shared/wire';
import { RunHub, RunOwner, DISPLAY_TOOLS } from './runs';
import type { RemoteClient } from './clients';

function fakeClient(id: number): RemoteClient & { sent: unknown[][]; destroy(): void } {
  let destroyed = false;
  const listeners: Array<() => void> = [];
  const sent: unknown[][] = [];
  return {
    id, kind: 'tab', sent,
    send(channel: string, ...args: unknown[]) { sent.push([channel, ...args]); },
    isDestroyed: () => destroyed,
    once: (_event: string, listener: () => void) => { listeners.push(listener); },
    destroy() { destroyed = true; for (const listener of listeners) listener(); }
  } as unknown as RemoteClient & { sent: unknown[][]; destroy(): void };
}

function harness(engine: RemoteClient | null, tabs: RemoteClient[]) {
  const responded: unknown[] = [];
  const hub = new RunHub({ engine: () => engine, tabs: () => tabs.filter((tab) => !tab.isDestroyed()), respond: (_owner, response) => responded.push(response) });
  return { hub, responded };
}

const request = { id: 'run-1', projectId: 'p1', mode: 'autonomous', prompt: 'do it' };

describe('RunOwner', () => {
  it('replays only what a resuming tab missed, and everything to a fresh one', () => {
    const tab = fakeClient(1);
    const { hub } = harness(null, [tab]);
    const owner = hub.begin(request, tab);
    for (const text of ['one', 'two', 'three']) owner.send(IPC.codexEvent, { id: 'run-1', kind: 'progress', text });
    tab.destroy();
    const resumed = fakeClient(2);
    owner.attach(resumed, { since: 2 });
    expect(resumed.sent.map((entry) => (entry[1] as { text: string }).text)).toEqual(['three']);
    const fresh = fakeClient(3);
    owner.attach(fresh, true);
    expect(fresh.sent.map((entry) => (entry[1] as { text: string }).text)).toEqual(['one', 'two', 'three']);
    hub.finish('run-1', { ok: true, text: 'done' } as never);
    const late = fakeClient(4);
    owner.attach(late, { since: 3 });
    expect(late.sent.map((entry) => entry[0])).toEqual([WEB.runFinished]);
  });

  it('buffers events, forwards them to attached tabs, and survives the tab', () => {
    const tab = fakeClient(1);
    const { hub } = harness(null, [tab]);
    const owner = hub.begin(request, tab);
    owner.send(IPC.codexEvent, { id: 'run-1', kind: 'progress', text: 'one' });
    tab.destroy();
    owner.send(IPC.codexEvent, { id: 'run-1', kind: 'progress', text: 'two' });
    expect(owner.isDestroyed()).toBe(false);
    expect(tab.sent).toEqual([[IPC.codexEvent, { id: 'run-1', kind: 'progress', text: 'one' }]]);
    expect(hub.record('run-1')?.events.map((event) => (event as { text: string }).text)).toEqual(['one', 'two']);
  });

  it('routes document tools to the engine and display tools to a tab', () => {
    const engine = fakeClient(9); engine.kind = 'engine';
    const tab = fakeClient(1);
    const { hub } = harness(engine, [tab]);
    const owner = hub.begin(request, tab);
    owner.send(IPC.agentToolRequest, { runId: 'run-1', callId: 'c1', tool: 'apply_commands', arguments: {}, baseRevision: 0 });
    owner.send(IPC.agentToolRequest, { runId: 'run-1', callId: 'c2', tool: 'capture_panel', arguments: {}, baseRevision: 0 });
    expect(engine.sent.map((entry) => (entry[1] as { callId: string; projectId: string }))).toMatchObject([{ callId: 'c1', projectId: 'p1' }]);
    expect(tab.sent.map((entry) => (entry[1] as { callId: string }).callId)).toEqual(['c2']);
    expect(hub.claimResponse('c1')).toBe(owner);
    expect(hub.claimResponse('c1')).toBeNull();
  });

  it('falls back to a tab for document tools without an engine, and fails display tools without a tab', () => {
    const tab = fakeClient(1);
    const { hub, responded } = harness(null, [tab]);
    const owner = hub.begin(request, tab);
    owner.send(IPC.agentToolRequest, { runId: 'run-1', callId: 'c1', tool: 'get_project_state', arguments: {}, baseRevision: 0 });
    expect(tab.sent).toHaveLength(1);
    tab.destroy();
    owner.send(IPC.agentToolRequest, { runId: 'run-1', callId: 'c2', tool: 'capture_panel', arguments: {}, baseRevision: 0 });
    expect(responded).toMatchObject([{ callId: 'c2', ok: false, error: expect.stringContaining('needs an open Powermove tab') }]);
  });

  it('records results and prunes finished runs after the retention window', () => {
    const tab = fakeClient(1);
    const { hub } = harness(null, [tab]);
    const owner = hub.begin(request, tab);
    hub.finish('run-1', { ok: true, text: 'done', access: 'project' });
    expect(hub.runsFor('p1')[0]?.result).toEqual({ ok: true, text: 'done', access: 'project' });
    const destroyed = vi.fn();
    owner.once('destroyed', destroyed);
    hub.prune(1000, Date.now() + 5000);
    expect(hub.owner('run-1')).toBeNull();
    expect(destroyed).toHaveBeenCalledOnce();
  });

  it('names the display tools the engine cannot serve', () => {
    expect(DISPLAY_TOOLS.has('capture_panel')).toBe(true);
    expect(DISPLAY_TOOLS.has('apply_commands')).toBe(false);
    expect(new RunOwner({ id: 'x', projectId: 'p', threadId: null, provider: 'compatible', mode: 'autonomous', prompt: '', startedAt: 0, finishedAt: null, events: [], result: null }, harness(null, []).hub).getURL()).toBe('powermove://run');
  });
});
