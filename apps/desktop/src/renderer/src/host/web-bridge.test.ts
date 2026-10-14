import { IPC } from '../../../shared/ipc';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { createBridge, uploadWording } from './web-bridge';

describe('upload toasts', () => {
  it('only says a file is opening when the person opened one', () => {
    const open = uploadWording('cut.mov', '12', 'open');
    expect(open.done).toBe('Uploaded cut.mov. Opening on the host…');
    const agent = uploadWording('cut.mov', '12', 'agent');
    expect([agent.start, agent.progress(40), agent.done, agent.failed('offline')]).toEqual([
      'Copying cut.mov (12 MB) to the host for the agent…',
      'Copying cut.mov for the agent… 40%',
      'Copied cut.mov to the host for the agent.',
      'Could not copy cut.mov to the host for the agent: offline'
    ]);
    expect([agent.start, agent.progress(1), agent.done, agent.failed('x')].some((line) => line.includes('Opening'))).toBe(false);
  });
});


afterEach(() => vi.unstubAllGlobals());

describe('remote lazy store', () => {
  function setup() {
    vi.stubGlobal('navigator', { userAgent: 'test' });
    vi.stubGlobal('window', {});
    vi.stubGlobal('document', { documentElement: { classList: { remove() {} } } });
    const events = new Map<string, Array<(payload?: unknown) => void>>();
    const link = {
      on: (key: string, cb: (payload?: unknown) => void) => { events.set(key, [...events.get(key) ?? [], cb]); return () => {}; },
      answer: vi.fn(), send: vi.fn(), invoke: vi.fn(async () => '{"fresh":true}'), sync: vi.fn(),
    };
    const bridge = createBridge(link as never, { node: 'test' } as never,
      { theme: '"dark"', __powermoveLazyKeys: '["project.keep","project.deleted","agentAttachment.image"]' },
      { projectId: null, taken: [] });
    return { link, bridge, emit: (key: string, value?: unknown) => events.get(key)?.forEach(cb => cb(value)) };
  }

  it('fetches only the requested lazy key asynchronously and supports legacy synchronous reads', async () => {
    const { bridge, link } = setup();
    expect(bridge.store.snapshotSerializedSync!()['project.keep']).toBeUndefined();
    expect(link.invoke).not.toHaveBeenCalled();
    expect(await bridge.store.getEncoded!('project.keep')).toBe('{"fresh":true}');
    expect(link.invoke).toHaveBeenCalledWith(IPC.storeGetEncoded, { key: 'project.keep' });
    expect(bridge.store.getSync!('project.keep')).toBe('{"fresh":true}');
    expect(bridge.store.snapshotSerializedSync!()['project.keep']).toBeUndefined();
    expect(JSON.parse(bridge.store.snapshotSerializedSync!().__powermoveLazyKeys!)).toContain('project.keep');
    const requests: string[] = [];
    vi.stubGlobal('XMLHttpRequest', class {
      status = 200; responseText = JSON.stringify('{"image":"pixels"}');
      open(method: string, url: string, async: boolean) { expect(method).toBe('GET'); expect(async).toBe(false); requests.push(url); }
      send() {}
    });
    expect(bridge.store.getSync!('agentAttachment.image')).toBe('{"image":"pixels"}');
    expect(requests).toEqual(['/__powermove/store?key=agentAttachment.image']);
    expect(bridge.store.getSync!('agentAttachment.image')).toBe('{"image":"pixels"}');
    expect(requests).toHaveLength(1);
  });

  it('invalidates lazy keys on reconnect without treating their missing bodies as deletions', async () => {
    const { bridge, link, emit } = setup();
    await bridge.store.getEncoded!('project.keep');
    await bridge.store.getEncoded!('project.deleted');
    link.sync.mockResolvedValue({ theme: '"light"', __powermoveLazyKeys: '["project.keep"]' });
    const changed = vi.fn(); bridge.store.onChanged!(changed);
    emit('__reconnected');
    await vi.waitFor(() => expect(changed).toHaveBeenCalled());
    expect(changed.mock.calls[0]![0]).toEqual(expect.arrayContaining(['project.keep', 'project.deleted', 'agentAttachment.image']));
    expect(bridge.store.getSync!('project.deleted')).toBeNull();
    expect(bridge.store.getSync!('theme')).toBe('"light"');
    link.invoke.mockResolvedValue('{"latest":true}');
    expect(await bridge.store.getEncoded!('project.keep')).toBe('{"latest":true}');
  });

  it('does not let an asynchronous read overwrite a newer local write or delete', async () => {
    const { bridge, link } = setup();
    let release!: (value: string) => void;
    link.invoke.mockImplementation(() => new Promise(resolve => { release = resolve; }));
    const read = bridge.store.getEncoded!('project.keep');
    bridge.store.set('project.keep', { edited: true }); release('{"old":true}');
    expect(await read).toBe('{"edited":true}');
    const deleted = bridge.store.getEncoded!('project.keep');
    bridge.store.delete('project.keep'); release('{"old":true}');
    expect(await deleted).toBeNull();
  });
});
