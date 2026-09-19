import { describe, expect, it, vi } from 'vitest';

import { ProjectSessions, type SessionMember } from './sessions';

function member(id: number): SessionMember & { sent: unknown[][]; destroy(): void } {
  let destroyed = false;
  const listeners: Array<() => void> = [];
  return {
    id,
    sent: [],
    send(channel, ...args) { this.sent.push([channel, ...args]); },
    isDestroyed: () => destroyed,
    once: (_event, listener) => { listeners.push(listener); },
    destroy() { destroyed = true; for (const listener of listeners) listener(); }
  };
}

function harness(slot: Record<string, unknown> = {}) {
  const store = { read: (key: string) => slot[key] ?? null, set: vi.fn((key: string, value: unknown) => { slot[key] = value; }) };
  const sessions = new ProjectSessions(store, { channel: 'sync', version: 't' });
  return { sessions, store, slot };
}

describe('ProjectSessions', () => {
  it('seeds from the first member and forwards patches to the others only', () => {
    const { sessions } = harness();
    const a = member(1), b = member(2);
    expect(sessions.join(a, 'p1', { id: 'p1', revision: 1, name: 'A' })).toEqual({ seq: 0, doc: null });
    // Same bytes as the session: nothing to send back.
    expect(sessions.join(b, 'p1', { id: 'p1', revision: 1, name: 'A' })).toEqual({ seq: 0, doc: null });
    const seq = sessions.patch(a, 'p1', [{ path: ['name'], exists: true, value: 'B' }]);
    expect(seq).toBe(1);
    expect(a.sent).toEqual([]);
    expect(b.sent).toEqual([['sync', { projectId: 'p1', seq: 1, patches: [{ path: ['name'], exists: true, value: 'B' }], from: 1 }]]);
    expect(sessions.document('p1')).toEqual({ id: 'p1', revision: 1, name: 'B' });
  });

  it('hands a late member the host copy when theirs differs', () => {
    const { sessions } = harness();
    const a = member(1), b = member(2);
    sessions.join(a, 'p1', { id: 'p1', revision: 0, layers: [] });
    sessions.patch(a, 'p1', [{ path: ['layers'], exists: true, value: [{ id: 'l1' }] }]);
    const result = sessions.join(b, 'p1', { id: 'p1', revision: 0, layers: [] });
    expect(result.seq).toBe(1);
    expect(result.doc).toEqual({ id: 'p1', revision: 0, layers: [{ id: 'l1' }] });
  });

  it('prefers the stored slot unless the member brings a newer revision', () => {
    const { sessions } = harness({ 'project.p1': { v: 't', proj: { id: 'p1', revision: 5, name: 'stored' } } });
    const a = member(1);
    expect(sessions.join(a, 'p1', { id: 'p1', revision: 2, name: 'stale' }).doc).toEqual({ id: 'p1', revision: 5, name: 'stored' });
    sessions.leave(a, 'p1');
    const b = member(2);
    expect(sessions.join(b, 'p1', { id: 'p1', revision: 9, name: 'newer' }).doc).toBeNull();
  });

  it('persists to the project slot after patches and on close', async () => {
    vi.useFakeTimers();
    try {
      const { sessions, store, slot } = harness();
      const a = member(1);
      sessions.join(a, 'p1', { id: 'p1', revision: 0, name: 'A' });
      sessions.patch(a, 'p1', [{ path: ['name'], exists: true, value: 'B' }]);
      expect(store.set).not.toHaveBeenCalled();
      vi.advanceTimersByTime(500);
      expect(slot['project.p1']).toEqual({ v: 't', proj: { id: 'p1', revision: 0, name: 'B' } });
      sessions.patch(a, 'p1', [{ path: ['name'], exists: true, value: 'C' }]);
      a.destroy();
      expect(slot['project.p1']).toEqual({ v: 't', proj: { id: 'p1', revision: 0, name: 'C' } });
      expect(sessions.members('p1')).toEqual([]);
    } finally { vi.useRealTimers(); }
  });

  it('rejects patches from non-members and malformed lists', () => {
    const { sessions } = harness();
    const a = member(1), b = member(2);
    sessions.join(a, 'p1', { id: 'p1' });
    expect(() => sessions.patch(b, 'p1', [{ path: ['x'], exists: true, value: 1 }])).toThrow(/not a member/);
    expect(() => sessions.patch(a, 'p1', [{ path: 'x' }])).toThrow(/invalid/);
    expect(() => sessions.patch(a, 'p1', [])).toThrow(/invalid/);
  });

  it('replace broadcasts a root patch to everyone but the origin', () => {
    const { sessions } = harness();
    const a = member(1), b = member(2);
    sessions.join(a, 'p1', { id: 'p1', n: 1 });
    sessions.join(b, 'p1', { id: 'p1', n: 1 });
    sessions.replace('p1', { id: 'p1', n: 2 }, null);
    expect(a.sent[0]![1]).toMatchObject({ seq: 1, patches: [{ path: [], exists: true, value: { id: 'p1', n: 2 } }] });
    expect(b.sent).toHaveLength(1);
  });
});
