import { afterEach, describe, expect, it, vi } from 'vitest';

import type { DefinitionsUpdate, Event, Request, Response } from '../../shared/engine-protocol';
import { INLINE_LIMIT, MAX_SVG_BAND_BYTES, WindowSnapshotSessions, type WindowSessionEngine } from './window-sessions';

const hash = 'ab'.repeat(32);
const json = (n = 7) => new TextEncoder().encode(JSON.stringify({ n }));

function fakeEngine() {
  const sent: Array<{ body: Request; session: number | null }> = [];
  const events: Array<{ body: Event; session: number | null }> = [];
  const staged: number[] = [];
  let launch = { port: 3, generation: 1 };
  let nextSession = 40;
  const engine: WindowSessionEngine = {
    isReady: () => true,
    hostPort: () => launch,
    request: async (_p, body, session) => {
      sent.push({ body, session: session ?? null });
      if (body.type === 'openSession') return { type: 'sessionOpened', session: nextSession++ } as unknown as Response;
      return { type: 'ack' } as unknown as Response;
    },
    stageDocument: async (_p, session, bytes) => {
      staged.push(bytes.byteLength);
      return { handle: `h${session}`, length: bytes.byteLength };
    },
    event: (_p, body, session) => events.push({ body, session })
  };
  return { engine, sent, events, staged, restart: (session?: number) => { launch = { port: 9, generation: 2 }; if (session !== undefined) nextSession = session; } };
}

function defs(generation: number | null = 4) {
  const state = { generation };
  return {
    state,
    relay: {
      generationOf: () => state.generation,
      snapshotFor: (_w: number, gen = 1): DefinitionsUpdate => ({ generation: gen, full: true, upserts: [], removals: [] })
    }
  };
}

afterEach(() => vi.useRealTimers());

describe('main’s snapshot sessions for windows', () => {
  it('opens one session per window, pushes its definitions, then resyncs the document', async () => {
    const f = fakeEngine();
    const d = defs();
    const sessions = new WindowSnapshotSessions(f.engine, d.relay, () => {});
    const got = await sessions.prepare(9, { id: 'doc', revision: 3, hash, json: json() });
    expect(got).toEqual({ port: 3, session: 40 });
    expect(f.sent.map((s) => s.body.type)).toEqual(['openSession', 'pushDefinitions', 'resync']);
    const resync = f.sent[2]!.body as unknown as { revision: number; source: { kind: string }; hash: Uint8Array };
    expect(resync.revision).toBe(3);
    expect(resync.source.kind).toBe('inline');
    expect(resync.hash).toEqual(new Uint8Array(32).fill(0xab));
    // The same revision again: nothing to send.
    await sessions.prepare(9, { id: 'doc', revision: 3 });
    expect(f.sent).toHaveLength(3);
    // A new revision without its body: the window must send it.
    expect(await sessions.prepare(9, { id: 'doc', revision: 4 })).toBe('needsDocument');
    // Definitions changed: pushed again, before the document.
    d.state.generation = 5;
    await sessions.prepare(9, { id: 'doc', revision: 4, hash, json: json(8) });
    expect(f.sent.slice(3).map((s) => s.body.type)).toEqual(['pushDefinitions', 'resync']);
    expect((f.sent[3]!.body as unknown as DefinitionsUpdate).generation).toBe(2);
    expect(sessions.stats).toMatchObject({ opened: 1, resyncs: 2, definitions: 2, needsDocument: 1 });
  });

  it('waits for the originating window font seeds before sending its document, and forgets them on close', async () => {
    const f = fakeEngine();
    let release!: () => void;
    const owner = vi.fn((_session: number, window: number | null) => window === null ? Promise.resolve() : new Promise<void>(r => { release = r; }));
    const sessions = new WindowSnapshotSessions(f.engine, defs().relay, () => {}, undefined, owner);
    const pending = sessions.prepare(9, { id: 'doc', revision: 1, hash, json: json() });
    await vi.waitFor(() => expect(owner).toHaveBeenCalledWith(40, 9));
    expect(f.sent.map(s => s.body.type)).toEqual(['openSession']);
    release();
    await pending;
    expect(f.sent.map(s => s.body.type)).toContain('resync');
    sessions.forget(9);
    expect(owner).toHaveBeenLastCalledWith(40, null);
  });

  it('refuses and closes a snapshot session whose window font seeds were not accepted', async () => {
    const f = fakeEngine();
    const owner = vi.fn(async (_session: number, window: number | null) => {
      if (window !== null) throw new Error('font seeds were refused');
    });
    const sessions = new WindowSnapshotSessions(f.engine, defs().relay, () => {}, undefined, owner);
    await expect(sessions.prepare(9, { id: 'doc', revision: 1, hash, json: json() })).rejects.toThrow('font seeds were refused');
    expect(sessions.has(9)).toBe(false);
    expect(f.sent.map(s => s.body.type)).toEqual(['openSession', 'closeSession']);
    expect(owner).toHaveBeenLastCalledWith(40, null);
  });

  it('forgets every old-launch font binding before session ids can be reused', async () => {
    const f = fakeEngine();
    const owners = new Map<number, number>();
    const owner = async (session: number, window: number | null) => {
      if (window === null) owners.delete(session);
      else owners.set(session, window);
    };
    const sessions = new WindowSnapshotSessions(f.engine, defs().relay, () => {}, undefined, owner);
    const doc = { id: 'doc', revision: 1, hash, json: json() };
    await sessions.prepare(9, doc);
    await sessions.prepare(10, doc);
    f.restart(41); // New window 9 reuses the old session id of window 10.
    await sessions.prepare(9, doc);
    await sessions.prepare(10, doc);
    expect(owners.get(41)).toBe(9);
    expect(owners.get(42)).toBe(10);
    expect(owners.has(40)).toBe(false);
    sessions.dispose();
  });

  it('stages a large document instead of sending it inline', async () => {
    const f = fakeEngine();
    const sessions = new WindowSnapshotSessions(f.engine, defs(null).relay, () => {});
    const big = new Uint8Array(INLINE_LIMIT + 1).fill(32);
    await sessions.prepare(9, { id: 'doc', revision: 1, hash, json: big });
    expect(f.staged).toEqual([INLINE_LIMIT + 1]);
    const resync = f.sent.find((s) => s.body.type === 'resync')!.body as unknown as { source: { kind: string; handle: string } };
    expect(resync.source).toMatchObject({ kind: 'cacheFile', handle: 'h40' });
  });

  it('posts fallback pixels to the session in bands before the snapshot', async () => {
    const f = fakeEngine();
    const sessions = new WindowSnapshotSessions(f.engine, defs().relay, () => {});
    await sessions.prepare(9, { id: 'doc', revision: 1, hash, json: json() }, [{ key: 'k'.repeat(64), width: 2, height: 3, rgba16f: new Uint8Array(2 * 3 * 8) }]);
    expect(f.events).toHaveLength(1);
    expect(f.events[0]).toMatchObject({ session: 40, body: { type: 'shaderFallback', fallback: 'pixels', key: 'k'.repeat(64), row: 0, rows: 3 } });
  });

  it('posts the window\'s compatibility producers and answers to its session before the snapshot', async () => {
    const f = fakeEngine();
    const sessions = new WindowSnapshotSessions(f.engine, defs().relay, () => {});
    const events = [
      { type: 'compat', compat: 'producers', sources: ['rasterOverride'] },
      { type: 'compat', compat: 'native', composition: 'c', layer: 'L', time: 0, source: 'rasterOverride', scale: 1, revision: 1 }
    ];
    await sessions.prepare(9, { id: 'doc', revision: 1, hash, json: json() }, [], [], events);
    expect(f.events.map((e: any) => e.body)).toEqual(events);
    expect(f.events.every((e: any) => e.session === 40)).toBe(true);
  });

  it('posts level 1 SVG rasters to the session in bands before the snapshot', async () => {
    const f = fakeEngine();
    const sessions = new WindowSnapshotSessions(f.engine, defs().relay, () => {});
    const width = 2048, height = 1100, rowBytes = width * 4;
    const key = { content: 'c'.repeat(64), width, height };
    const data = new Uint8Array(rowBytes * height);
    data[rowBytes * 1024] = 9;
    await sessions.prepare(9, { id: 'doc', revision: 1, hash, json: json() }, [], [{ key, data }]);
    const rows = Math.floor(MAX_SVG_BAND_BYTES / rowBytes);
    expect(f.events.map((e) => [(e.body as any).row, (e.body as any).rows])).toEqual([[0, rows], [rows, height - rows]]);
    expect(f.events[1]).toMatchObject({ session: 40, body: { type: 'compat', compat: 'svgPixels', key } });
    expect((f.events[1]!.body as any).data[0]).toBe(9);
    // Pixels that do not match their size are dropped.
    await sessions.prepare(9, { id: 'doc', revision: 1 }, [], [{ key, data: new Uint8Array(4) }]);
    expect(f.events).toHaveLength(2);
  });

  it('opens a new session after an engine restart, and asks for the document again', async () => {
    const f = fakeEngine();
    const sessions = new WindowSnapshotSessions(f.engine, defs().relay, () => {});
    await sessions.prepare(9, { id: 'doc', revision: 1, hash, json: json() });
    f.restart();
    expect(await sessions.prepare(9, { id: 'doc', revision: 1 })).toBe('needsDocument');
    expect(await sessions.prepare(9, { id: 'doc', revision: 1, hash, json: json() })).toEqual({ port: 9, session: 41 });
  });

  it('closes a window’s session when the window goes and after it is unused', async () => {
    vi.useFakeTimers();
    const f = fakeEngine();
    const sessions = new WindowSnapshotSessions(f.engine, defs().relay, () => {}, 1000);
    await sessions.prepare(9, { id: 'doc', revision: 1, hash, json: json() });
    await sessions.prepare(10, { id: 'doc2', revision: 1, hash, json: json() });
    sessions.forget(9);
    expect(sessions.has(9)).toBe(false);
    expect(f.sent.filter((s) => s.body.type === 'closeSession').map((s) => s.session)).toEqual([40]);
    vi.advanceTimersByTime(1001);
    expect(sessions.has(10)).toBe(false);
    expect(f.sent.filter((s) => s.body.type === 'closeSession').map((s) => s.session)).toEqual([40, 41]);
  });

  it('serializes a window’s requests', async () => {
    const f = fakeEngine();
    const sessions = new WindowSnapshotSessions(f.engine, defs().relay, () => {});
    const [a, b] = await Promise.all([
      sessions.prepare(9, { id: 'doc', revision: 1, hash, json: json() }),
      sessions.prepare(9, { id: 'doc', revision: 1 })
    ]);
    expect(a).toEqual(b);
    expect(f.sent.filter((s) => s.body.type === 'openSession')).toHaveLength(1);
  });
});
