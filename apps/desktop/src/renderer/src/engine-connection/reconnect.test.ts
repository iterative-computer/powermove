import { describe, expect, it } from 'vitest';
import { PortReconnector, RECONNECT_FIRST_MS, RECONNECT_MAX_MS, RECONNECT_RESET_MS, RECONNECT_WAIT_MS, type ReconnectClock } from './reconnect';

/** A clock whose timers run when the test moves time. */
function fakeClock() {
  let now = 0;
  let timers: Array<{ at: number; fn: () => void; id: number }> = [];
  let ids = 0;
  const clock: ReconnectClock & { advance(ms: number): void; pending(): number } = {
    now: () => now,
    later: (fn, ms) => { const id = ++ids; timers.push({ at: now + ms, fn, id }); return id; },
    cancel: (id) => { timers = timers.filter((t) => t.id !== id); },
    advance(ms) {
      const until = now + ms;
      for (;;) {
        timers.sort((a, b) => a.at - b.at);
        const next = timers[0];
        if (!next || next.at > until) break;
        timers.shift();
        now = next.at;
        next.fn();
      }
      now = until;
    },
    pending: () => timers.length
  };
  return clock;
}

describe('reconnecting a window whose engine connection closed', () => {
  it('asks main for ports with the supervisor backoff until one connects', () => {
    const clock = fakeClock();
    const asks: number[] = [];
    let connected = false;
    const r = new PortReconnector(() => asks.push(clock.now()), () => connected, clock);
    r.opened();
    clock.advance(1000);
    r.closed();
    clock.advance(RECONNECT_FIRST_MS - 1);
    expect(asks).toEqual([]);
    clock.advance(1);
    expect(asks).toEqual([1000 + RECONNECT_FIRST_MS]);
    // No port: asks again after the wait plus the doubled delay.
    clock.advance(RECONNECT_WAIT_MS + 2 * RECONNECT_FIRST_MS);
    expect(asks).toHaveLength(2);
    // A port arrives and connects: nothing more is asked.
    r.portArrived();
    connected = true;
    r.opened();
    clock.advance(RECONNECT_MAX_MS * 4);
    expect(asks).toHaveLength(2);
    expect(clock.pending()).toBe(0);
  });

  it('caps the delay at 30 s and resets it after a connection lasted a minute', () => {
    const clock = fakeClock();
    let asks = 0;
    const r = new PortReconnector(() => asks++, () => false, clock);
    r.closed();
    for (let i = 0; i < 12; i++) clock.advance(RECONNECT_MAX_MS + RECONNECT_WAIT_MS);
    const before = asks;
    clock.advance(RECONNECT_MAX_MS + RECONNECT_WAIT_MS);
    expect(asks - before).toBe(1);
    // A connection that lasts a minute starts the backoff over.
    r.portArrived();
    r.opened();
    clock.advance(RECONNECT_RESET_MS);
    const at = clock.now();
    let first = -1;
    const timed = new PortReconnector(() => { if (first < 0) first = clock.now(); }, () => false, clock);
    timed.opened();
    clock.advance(RECONNECT_RESET_MS);
    timed.closed();
    clock.advance(RECONNECT_FIRST_MS);
    expect(first - at).toBe(RECONNECT_RESET_MS + RECONNECT_FIRST_MS);
    r.dispose();
    timed.dispose();
  });

  it('asks nothing once a new launch\'s port arrived, or after disposal', () => {
    const clock = fakeClock();
    let asks = 0;
    const r = new PortReconnector(() => asks++, () => false, clock);
    r.closed();
    r.portArrived();
    clock.advance(RECONNECT_MAX_MS);
    expect(asks).toBe(0);
    r.closed();
    r.dispose();
    clock.advance(RECONNECT_MAX_MS);
    expect(asks).toBe(0);
  });
});
