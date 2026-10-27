import { describe, expect, it } from 'vitest';
import { SnapshotSlots } from './snapshot-slots';

const deferred = () => { let resolve!: () => void; const promise = new Promise<void>((r) => { resolve = r; }); return { promise, resolve }; };

describe('snapshot slots', () => {
  it('runs at most the capacity at once and queues the rest, the most urgent class first', async () => {
    const slots = new SnapshotSlots(2);
    const order: string[] = [];
    const gates = new Map<string, ReturnType<typeof deferred>>();
    const job = (cls: 'sync' | 'async' | 'idle', name: string) => slots.run(cls, async () => {
      order.push(name);
      const gate = deferred(); gates.set(name, gate); await gate.promise; return name;
    });
    const all = [job('idle', 'i1'), job('idle', 'i2'), job('idle', 'i3'), job('async', 'a1'), job('sync', 's1')];
    await Promise.resolve(); await Promise.resolve();
    expect(order).toEqual(['i1', 'i2']);
    expect(slots.inFlight).toBe(2);
    expect(slots.queued).toBe(3);
    gates.get('i1')!.resolve();
    await new Promise((r) => setTimeout(r, 0));
    expect(order).toEqual(['i1', 'i2', 's1']);
    gates.get('i2')!.resolve();
    await new Promise((r) => setTimeout(r, 0));
    expect(order).toEqual(['i1', 'i2', 's1', 'a1']);
    gates.get('s1')!.resolve(); gates.get('a1')!.resolve();
    await new Promise((r) => setTimeout(r, 0));
    gates.get('i3')!.resolve();
    expect(await Promise.all(all)).toEqual(['i1', 'i2', 'i3', 'a1', 's1']);
    expect(slots.inFlight).toBe(0);
  });

  it('keeps the results in flight within main\'s byte reservation, in order, so main never refuses one', async () => {
    const slots = new SnapshotSlots(8, 100);
    const order: string[] = [];
    const gates = new Map<string, ReturnType<typeof deferred>>();
    const job = (name: string, bytes: number) => slots.run('sync', async () => {
      order.push(name);
      const gate = deferred(); gates.set(name, gate); await gate.promise; return name;
    }, bytes);
    const all = [job('a', 40), job('b', 40), job('c', 40), job('d', 10), job('e', 500)];
    await Promise.resolve(); await Promise.resolve();
    // c does not fit beside a and b; d waits behind it.
    expect(order).toEqual(['a', 'b']);
    gates.get('a')!.resolve();
    await new Promise((r) => setTimeout(r, 0));
    expect(order).toEqual(['a', 'b', 'c', 'd']);
    gates.get('b')!.resolve(); gates.get('c')!.resolve(); gates.get('d')!.resolve();
    await new Promise((r) => setTimeout(r, 0));
    // One larger than the whole reservation runs alone.
    expect(order).toEqual(['a', 'b', 'c', 'd', 'e']);
    gates.get('e')!.resolve();
    expect(await Promise.all(all)).toEqual(['a', 'b', 'c', 'd', 'e']);
    expect(slots.inFlight).toBe(0);
  });

  it('frees a slot when a request fails', async () => {
    const slots = new SnapshotSlots(1);
    await expect(slots.run('async', async () => { throw new Error('engine said no'); })).rejects.toThrow('engine said no');
    expect(await slots.run('async', async () => 7)).toBe(7);
    expect(slots.inFlight).toBe(0);
  });
});
