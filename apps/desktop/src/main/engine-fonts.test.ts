import { existsSync, readFileSync } from 'node:fs';
import path from 'node:path';
import { describe, expect, it, vi } from 'vitest';
import {
  ENGINE_FONTS_IPC,
  bridgeAddonPath,
  engineFontsEnabled,
  loadNativeFontRegistry,
  registerEngineFontsIpc,
  setupEngineFonts,
  type NativeFontRegistry
} from './engine-fonts';

function fakeRegistry(): NativeFontRegistry & { calls: unknown[][] } {
  const calls: unknown[][] = [];
  let generation = 0;
  return {
    calls,
    get generation() {
      return generation;
    },
    familyNames: () => ['Georgia'],
    familyFaces: family => (calls.push(['familyFaces', family]), '[]'),
    fallbackManifest: base => (calls.push(['fallbackManifest', base]), `{"baseFamily":"${base}"}`),
    addProjectFont(family, data, descriptors, index) {
      calls.push(['addProjectFont', family, data.byteLength, descriptors, index]);
      if (descriptors?.weight === 'heavy') throw new Error('invalid FontFace weight descriptor "heavy"');
      generation++;
    },
    setFontSource(family, data) {
      calls.push(['setFontSource', family, data.byteLength]);
      generation++;
    },
    clearProjectFonts() {
      calls.push(['clearProjectFonts']);
      generation++;
    },
    answer: queries => (calls.push(['answer', queries]), '{"generation":0,"faces":[],"answers":[]}'),
    subset: (blob, index) => (calls.push(['subset', blob, index]), new Uint8Array([0, 1, 0, 0]))
  };
}

function register(registry: NativeFontRegistry) {
  const handlers = new Map<string, (event: unknown, ...args: unknown[]) => unknown>();
  const trusted = {};
  registerEngineFontsIpc({ handle: (channel: string, fn: any) => void handlers.set(channel, fn) } as any, { isTrustedSender: e => e.sender === trusted }, registry);
  const call = (channel: string, ...args: unknown[]) => handlers.get(channel)!({ sender: trusted }, ...args);
  const untrusted = (channel: string, ...args: unknown[]) => handlers.get(channel)!({ sender: {} }, ...args);
  return { handlers, call, untrusted };
}

const BLOB = '0123456789abcdef0123456789abcdef';

describe('engine fonts flag', () => {
  it('is off unless POWERMOVE_ENGINE_FONTS=1, and then registers nothing', () => {
    expect(engineFontsEnabled({})).toBe(false);
    expect(engineFontsEnabled({ POWERMOVE_ENGINE_FONTS: 'true' })).toBe(false);
    expect(engineFontsEnabled({ POWERMOVE_ENGINE_FONTS: '1' })).toBe(true);
    const handle = vi.fn();
    const load = vi.fn();
    expect(setupEngineFonts({ ipc: { handle }, ctx: { isTrustedSender: () => true }, env: {}, appPath: '/app', load })).toBeNull();
    expect(handle).not.toHaveBeenCalled();
    expect(load).not.toHaveBeenCalled();
  });

  it('finds the development addon next to the app unless overridden', () => {
    expect(bridgeAddonPath({}, '/repo/apps/desktop')).toBe('/repo/engine/target/pm-bridge/pm_bridge.node');
    expect(bridgeAddonPath({ POWERMOVE_PM_BRIDGE: '/x/pm.node' }, '/repo/apps/desktop')).toBe('/x/pm.node');
  });

  it('logs and carries on when the addon is missing or broken', () => {
    const handle = vi.fn();
    const log = vi.fn();
    const ctx = { isTrustedSender: () => true };
    const env = { POWERMOVE_ENGINE_FONTS: '1', POWERMOVE_PM_BRIDGE: '/nonexistent/pm_bridge.node' };
    expect(setupEngineFonts({ ipc: { handle }, ctx, env, appPath: '/app', log })).toBeNull();
    expect(log.mock.calls[0]![0]).toMatch(/addon is missing/);
    const present = { ...env, POWERMOVE_PM_BRIDGE: __filename };
    expect(setupEngineFonts({ ipc: { handle }, ctx, env: present, appPath: '/app', log, load: () => ({}) })).toBeNull();
    expect(log.mock.calls[1]![0]).toMatch(/does not export FontRegistry/);
    expect(handle).not.toHaveBeenCalled();
  });

  it('registers every channel when the addon loads', () => {
    const handle = vi.fn();
    const registry = fakeRegistry();
    const FontRegistry = vi.fn(function () {
      return registry;
    });
    const env = { POWERMOVE_ENGINE_FONTS: '1', POWERMOVE_PM_BRIDGE: __filename };
    expect(setupEngineFonts({ ipc: { handle }, ctx: { isTrustedSender: () => true }, env, appPath: '/app', load: () => ({ FontRegistry }) })).toBe(registry);
    expect(handle.mock.calls.map(c => c[0]).sort()).toEqual(Object.values(ENGINE_FONTS_IPC).sort());
  });
});

describe('engine fonts channels', () => {
  it('answer only trusted senders', () => {
    const registry = fakeRegistry();
    const { handlers, untrusted } = register(registry);
    for (const channel of handlers.keys()) expect(untrusted(channel, '[]')).toBeNull();
    expect(registry.calls).toEqual([]);
  });

  it('forward pm-text JSON untouched', () => {
    const registry = fakeRegistry();
    const { call } = register(registry);
    const queries = '[{"kind":"family","name":"Georgia","weight":1137180672,"italic":false,"size":1098907648}]';
    expect(call(ENGINE_FONTS_IPC.answer, queries)).toEqual({ ok: true, value: '{"generation":0,"faces":[],"answers":[]}' });
    expect(registry.calls[0]).toEqual(['answer', queries]);
    expect(call(ENGINE_FONTS_IPC.manifest, 'system-ui')).toEqual({ ok: true, value: '{"baseFamily":"system-ui"}' });
    expect(call(ENGINE_FONTS_IPC.familyFaces, 'Georgia')).toEqual({ ok: true, value: '[]' });
  });

  it('validate subset keys, families, and font bytes', () => {
    const registry = fakeRegistry();
    const { call } = register(registry);
    expect(call(ENGINE_FONTS_IPC.subset, BLOB, 0)).toEqual({ ok: true, value: new Uint8Array([0, 1, 0, 0]) });
    for (const [blob, index] of [['../etc', 0], [BLOB.toUpperCase(), 0], [BLOB, -1], [BLOB, 1.5], [BLOB, '0']]) {
      expect(call(ENGINE_FONTS_IPC.subset, blob, index)).toEqual({ ok: false, error: 'invalid subset key' });
    }
    expect(call(ENGINE_FONTS_IPC.answer, '')).toEqual({ ok: false, error: 'invalid queries' });
    expect(call(ENGINE_FONTS_IPC.answer, 7)).toEqual({ ok: false, error: 'invalid queries' });
    expect(call(ENGINE_FONTS_IPC.manifest, 'x'.repeat(513))).toEqual({ ok: false, error: 'invalid family' });
    expect(call(ENGINE_FONTS_IPC.addProjectFont, 'Brand', new Uint8Array(0))).toEqual({ ok: false, error: 'invalid project font' });
    expect(call(ENGINE_FONTS_IPC.addProjectFont, 'Brand', new Uint8Array(4), { weight: 400 })).toEqual({ ok: false, error: 'invalid project font' });
    expect(call(ENGINE_FONTS_IPC.setFontSource, 'Brand', 'bytes')).toEqual({ ok: false, error: 'invalid font source' });
    expect(registry.calls.filter(c => c[0] !== 'subset')).toEqual([]);
  });

  it('register project fonts and report the new generation', () => {
    const registry = fakeRegistry();
    const { call } = register(registry);
    expect(call(ENGINE_FONTS_IPC.addProjectFont, 'Brand', new Uint8Array(8).buffer, { weight: '100 900', style: 'italic' })).toEqual({ ok: true, value: 1 });
    expect(registry.calls.at(-1)).toEqual(['addProjectFont', 'Brand', 8, { weight: '100 900', style: 'italic' }, undefined]);
    expect(call(ENGINE_FONTS_IPC.addProjectFont, 'Brand', new Uint8Array(8), undefined, 2)).toEqual({ ok: true, value: 2 });
    expect(call(ENGINE_FONTS_IPC.addProjectFont, 'Brand', new Uint8Array(8), { weight: 'heavy' })).toEqual({ ok: false, error: 'invalid FontFace weight descriptor "heavy"' });
    expect(call(ENGINE_FONTS_IPC.setFontSource, 'Brand', new Uint16Array(4))).toEqual({ ok: true, value: 3 });
    expect(registry.calls.at(-1)).toEqual(['setFontSource', 'Brand', 8]);
    expect(call(ENGINE_FONTS_IPC.clearProjectFonts)).toEqual({ ok: true, value: 4 });
    expect(call(ENGINE_FONTS_IPC.generation)).toEqual({ ok: true, value: 4 });
  });
});

// The real addon, when a development build exists (engine/crates/pm-bridge/scripts/build.sh).
const addon = bridgeAddonPath({}, path.resolve(__dirname, '../..'));
describe.skipIf(process.platform !== 'darwin' || !existsSync(addon))('engine fonts with the pm-bridge addon', () => {
  it('answers, serves subsets, and registers project fonts', () => {
    const registry = loadNativeFontRegistry(addon);
    const { call } = register(registry);
    const bits = (n: number) => new Uint32Array(new Float32Array([n]).buffer)[0]!;
    const queries = JSON.stringify([{ kind: 'family', name: 'Georgia', weight: bits(400), italic: false, size: bits(16) }]);
    const reply = call(ENGINE_FONTS_IPC.answer, queries) as { ok: true; value: string };
    const answers = JSON.parse(reply.value);
    expect(answers.faces[0].postscriptName).toBe('Georgia');
    const subset = call(ENGINE_FONTS_IPC.subset, answers.faces[0].blob, answers.faces[0].index) as { ok: true; value: Uint8Array };
    expect(subset.ok).toBe(true);
    expect(subset.value.byteLength).toBeGreaterThan(1000);
    expect(call(ENGINE_FONTS_IPC.subset, BLOB, 0)).toMatchObject({ ok: false, error: expect.stringMatching(/unknown font blob/) });
    const georgia = readFileSync('/System/Library/Fonts/Supplemental/Georgia.ttf');
    const before = registry.generation;
    expect(call(ENGINE_FONTS_IPC.addProjectFont, 'Brand Test', georgia, { weight: '400' })).toEqual({ ok: true, value: before + 1 });
    expect(call(ENGINE_FONTS_IPC.addProjectFont, 'Brand Test', georgia, { weight: 'heavy' })).toMatchObject({ ok: false });
    expect(call(ENGINE_FONTS_IPC.addProjectFont, 'Brand Test', new Uint8Array([1, 2, 3, 4]))).toMatchObject({ ok: false });
    expect(call(ENGINE_FONTS_IPC.clearProjectFonts)).toEqual({ ok: true, value: before + 2 });
  });
});

/* ── The engine's font service (Use New Engine) ───────────────────────── */

import {
  EngineFontClient,
  FONT_OP,
  FONT_STATUS,
  answerGeneration,
  decodeFontReply,
  encodeFontRequest,
  installEngineFontService
} from './engine-fonts';
import type { PortLike } from './engine/ports';

/** One end of a font port: records what main posts, and lets the test
 *  answer as the engine would. */
class FakeFontPort implements PortLike {
  sent: { id: number; op: number; json: string; tail: Uint8Array }[] = [];
  closed = false;
  started = false;
  private message: ((event: { data: unknown }) => void) | null = null;
  private onClose: (() => void) | null = null;
  postMessage(data: unknown): void {
    const bytes = data as Uint8Array;
    const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
    const length = view.getUint32(5, true);
    this.sent.push({ id: view.getUint32(0, true), op: bytes[4]!, json: new TextDecoder().decode(bytes.subarray(9, 9 + length)), tail: bytes.slice(9 + length) });
  }
  on(event: 'message' | 'close', listener: any): unknown {
    if (event === 'message') this.message = listener;
    else this.onClose = listener;
    return this;
  }
  start(): void {
    this.started = true;
  }
  close(): void {
    this.closed = true;
  }
  /** The engine's reply to request `id`. */
  reply(id: number, status: number, payload: Uint8Array | string): void {
    const body = typeof payload === 'string' ? new TextEncoder().encode(payload) : payload;
    const out = new Uint8Array(5 + body.byteLength);
    new DataView(out.buffer).setUint32(0, id, true);
    out[4] = status;
    out.set(body, 5);
    this.message?.({ data: out });
  }
  /** Answers every request not answered yet like a fresh font service. */
  serve(state: { generation: number }): void {
    for (const r of this.sent.splice(0)) {
      if (r.op === FONT_OP.open) state.generation = JSON.parse(r.json).generationBase;
      else if (r.op === FONT_OP.addProjectFont || r.op === FONT_OP.setFontSource || r.op === FONT_OP.clearProjectFonts) state.generation++;
      if (r.op === FONT_OP.subset) this.reply(r.id, FONT_STATUS.bytes, new Uint8Array([0, 1, 0, 0]));
      else if (r.op === FONT_OP.answer) this.reply(r.id, FONT_STATUS.json, `{"answers":[],"faces":[],"generation":${state.generation}}`);
      else this.reply(r.id, FONT_STATUS.json, `{"generation":${state.generation}}`);
    }
  }
  goAway(): void {
    this.onClose?.();
  }
}

const manualSchedule = () => {
  const timers: { fn: () => void; ms: number; cancelled: boolean }[] = [];
  return {
    timers,
    schedule: (fn: () => void, ms: number) => {
      const t = { fn, ms, cancelled: false };
      timers.push(t);
      return { cancel: () => void (t.cancelled = true) };
    },
    fire: () => timers.filter(t => !t.cancelled).forEach(t => t.fn())
  };
};

describe('the font port wire format', () => {
  it('matches pm-node fonts::wire byte for byte', () => {
    // pm-node `golden_bytes_match_mains_codec`.
    expect([...encodeFontRequest(0x01020304, FONT_OP.subset, '{"blob":"ab","index":1}')]).toEqual([4, 3, 2, 1, 3, 23, 0, 0, 0, ...new TextEncoder().encode('{"blob":"ab","index":1}')]);
    expect(decodeFontReply(new Uint8Array([2, 1, 0, 0, 2, 110, 111]))).toEqual({ id: 258, status: FONT_STATUS.error, payload: new Uint8Array([110, 111]) });
    expect(decodeFontReply(new Uint8Array([0, 0, 0, 0, 7]))).toBeNull();
    expect(decodeFontReply('nope')).toBeNull();
    const tail = encodeFontRequest(5, FONT_OP.addProjectFont, '{}', new Uint8Array([9, 8]));
    expect([...tail.subarray(-2)]).toEqual([9, 8]);
  });

  it('reads the generation off the end of an answer', () => {
    expect(answerGeneration('{"answers":[{"x":1}],"faces":[],"generation":42}')).toBe(42);
    expect(answerGeneration('{"generation":3,"answers":[]}')).toBeNull();
  });
});

describe('the engine font client', () => {
  it('opens every port above every generation seen, then replays project fonts in order', async () => {
    const client = new EngineFontClient();
    const engine = { generation: 0 };
    const first = new FakeFontPort();
    client.connect(first);
    expect(first.started).toBe(true);
    expect(first.sent[0]).toMatchObject({ op: FONT_OP.open, json: '{"generationBase":1}' });
    first.serve(engine);
    const added = client.addProjectFont('Brand', new Uint8Array([1, 2]), { weight: '400' }, undefined);
    const source = client.setFontSource('Brand', new Uint8Array([3]));
    expect(first.sent.map(r => [r.op, r.json, [...r.tail]])).toEqual([
      [FONT_OP.addProjectFont, '{"family":"Brand","weight":"400"}', [1, 2]],
      [FONT_OP.setFontSource, '{"family":"Brand"}', [3]]
    ]);
    first.serve(engine);
    expect(await added).toBe(2);
    expect(await source).toBe(3);

    // The engine restarts: a new port, a higher base, the same project fonts.
    first.goAway();
    const second = new FakeFontPort();
    client.connect(second);
    expect(second.sent.map(r => [r.op, r.json])).toEqual([
      [FONT_OP.open, '{"generationBase":4}'],
      [FONT_OP.addProjectFont, '{"family":"Brand","weight":"400"}'],
      [FONT_OP.setFontSource, '{"family":"Brand"}']
    ]);
    second.serve(engine);
    expect(client.generationSeen).toBe(6);
    // A clear empties the replay.
    const cleared = client.clearProjectFonts();
    second.serve(engine);
    expect(await cleared).toBe(7);
    second.goAway();
    const third = new FakeFontPort();
    client.connect(third);
    expect(third.sent.map(r => r.op)).toEqual([FONT_OP.open]);
    expect(third.sent[0]!.json).toBe('{"generationBase":8}');
  });

  it('splices the renderer\'s queries in untouched and passes the generation it saw', async () => {
    const client = new EngineFontClient();
    const port = new FakeFontPort();
    client.connect(port);
    port.serve({ generation: 0 });
    const queries = '[{"kind":"family","name":"Georgia","weight":1137180672,"italic":false,"size":1098907648}]';
    const answer = client.answer(queries, 1);
    expect(port.sent[0]!.json).toBe(`{"queries":${queries},"known":1}`);
    port.serve({ generation: 1 });
    expect(await answer).toBe('{"answers":[],"faces":[],"generation":1}');
    void client.answer('[]', null);
    expect(port.sent[0]!.json).toBe('{"queries":[]}');
    const subset = client.subset('0123456789abcdef0123456789abcdef', 2);
    expect(JSON.parse(port.sent[1]!.json)).toEqual({ blob: '0123456789abcdef0123456789abcdef', index: 2 });
    port.serve({ generation: 1 });
    expect([...(await subset)]).toEqual([0, 1, 0, 0]);
  });

  it('queues requests while the engine is down and sends read-only ones again after a restart', async () => {
    const client = new EngineFontClient();
    const queued = client.manifest('system-ui');
    const first = new FakeFontPort();
    client.connect(first);
    expect(first.sent.map(r => r.op)).toEqual([FONT_OP.open, FONT_OP.manifest]);
    // The engine dies before answering.
    first.goAway();
    expect(client.connected).toBe(false);
    const second = new FakeFontPort();
    client.connect(second);
    expect(second.sent.map(r => r.op)).toEqual([FONT_OP.open, FONT_OP.manifest]);
    second.serve({ generation: 0 });
    expect(await queued).toBe('{"generation":1}');
    // Replies on a port that was replaced are ignored.
    first.reply(2, FONT_STATUS.json, '{"generation":99}');
    expect(client.generationSeen).toBe(1);
  });

  it('settles an in-flight project font with the replay\'s generation after a restart', async () => {
    const client = new EngineFontClient();
    const first = new FakeFontPort();
    client.connect(first);
    first.serve({ generation: 0 });
    const added = client.addProjectFont('Brand', new Uint8Array([1]), {}, 2);
    expect(first.sent[0]!.json).toBe('{"family":"Brand","index":2}');
    first.goAway();
    const second = new FakeFontPort();
    client.connect(second);
    expect(second.sent.map(r => r.op)).toEqual([FONT_OP.open, FONT_OP.addProjectFont, FONT_OP.generation]);
    second.serve({ generation: 0 });
    // Base 2 (above the 1 seen), then the replayed font.
    expect(await added).toBe(3);
  });

  it('sends a project font made while the engine is down once, after the replay', async () => {
    const client = new EngineFontClient();
    const engine = { generation: 0 };
    const first = new FakeFontPort();
    client.connect(first);
    first.serve(engine);
    const before = client.addProjectFont('Old', new Uint8Array([1]), {}, undefined);
    first.serve(engine);
    expect(await before).toBe(2);
    first.goAway();
    // The engine is restarting: one font added, then the project's fonts
    // cleared and another added, all before the new port connects.
    const added = client.addProjectFont('Brand', new Uint8Array([2]), {}, undefined);
    const source = client.setFontSource('Brand', new Uint8Array([3]));
    const second = new FakeFontPort();
    client.connect(second);
    expect(second.sent.map(r => [r.op, r.json])).toEqual([
      [FONT_OP.open, '{"generationBase":3}'],
      [FONT_OP.addProjectFont, '{"family":"Old"}'],
      [FONT_OP.addProjectFont, '{"family":"Brand"}'],
      [FONT_OP.setFontSource, '{"family":"Brand"}']
    ]);
    second.serve(engine);
    expect(await added).toBe(5);
    expect(await source).toBe(6);
    second.goAway();
    const cleared = client.clearProjectFonts();
    const after = client.addProjectFont('New', new Uint8Array([4]), {}, undefined);
    const third = new FakeFontPort();
    client.connect(third);
    expect(third.sent.map(r => [r.op, r.json])).toEqual([
      [FONT_OP.open, '{"generationBase":7}'],
      [FONT_OP.addProjectFont, '{"family":"Old"}'],
      [FONT_OP.addProjectFont, '{"family":"Brand"}'],
      [FONT_OP.setFontSource, '{"family":"Brand"}'],
      [FONT_OP.clearProjectFonts, '{}'],
      [FONT_OP.addProjectFont, '{"family":"New"}']
    ]);
    third.serve(engine);
    expect(await cleared).toBe(11);
    expect(await after).toBe(12);
    // The replay now holds only what followed the clear.
    third.goAway();
    const fourth = new FakeFontPort();
    client.connect(fourth);
    expect(fourth.sent.map(r => [r.op, r.json])).toEqual([
      [FONT_OP.open, '{"generationBase":13}'],
      [FONT_OP.addProjectFont, '{"family":"New"}']
    ]);
  });

  it('keeps a change that timed out after it went out in the replay unless the engine refuses it', async () => {
    const timers = manualSchedule();
    const client = new EngineFontClient({ timeoutMs: 50, schedule: timers.schedule });
    const port = new FakeFontPort();
    client.connect(port);
    port.serve({ generation: 0 });
    const slow = client.addProjectFont('Slow', new Uint8Array([1]), {}, undefined);
    const refused = client.addProjectFont('Bad', new Uint8Array([2]), {}, undefined);
    const [slowReq, badReq] = port.sent.splice(0);
    timers.fire();
    await expect(slow).rejects.toThrow(/did not answer within 50 ms/);
    await expect(refused).rejects.toThrow(/did not answer within 50 ms/);
    // The engine gets to them late: it applies one and refuses the other.
    port.reply(slowReq!.id, FONT_STATUS.json, '{"generation":2}');
    port.reply(badReq!.id, FONT_STATUS.error, 'not a font');
    expect(client.generationSeen).toBe(2);
    port.goAway();
    const next = new FakeFontPort();
    client.connect(next);
    expect(next.sent.map(r => [r.op, r.json])).toEqual([
      [FONT_OP.open, '{"generationBase":3}'],
      [FONT_OP.addProjectFont, '{"family":"Slow"}']
    ]);
  });

  it('drops a refused change from the replay and reports the engine\'s error', async () => {
    const log = vi.fn();
    const client = new EngineFontClient({ log });
    const port = new FakeFontPort();
    client.connect(port);
    port.serve({ generation: 0 });
    const refused = client.addProjectFont('Bad', new Uint8Array([1, 2, 3]), {}, undefined);
    port.reply(port.sent[0]!.id, FONT_STATUS.error, 'not a font');
    await expect(refused).rejects.toThrow('not a font');
    port.goAway();
    const next = new FakeFontPort();
    client.connect(next);
    expect(next.sent.map(r => r.op)).toEqual([FONT_OP.open]);
    // Malformed messages are dropped with a note.
    (next as any).message({ data: new Uint8Array([1]) });
    expect(log.mock.calls.at(-1)![1]).toMatch(/malformed/);
  });

  it('fails requests the engine never answers, and every waiting request on stop', async () => {
    const timers = manualSchedule();
    const client = new EngineFontClient({ timeoutMs: 50, schedule: timers.schedule });
    const waiting = client.generation();
    timers.fire();
    await expect(waiting).rejects.toThrow(/did not answer within 50 ms/);
    const port = new FakeFontPort();
    client.connect(port);
    expect(port.sent.map(r => r.op)).toEqual([FONT_OP.open]);
    const pending = client.familyFaces('Georgia');
    client.disconnect();
    await expect(pending).rejects.toThrow(/stopped/);
    expect(port.closed).toBe(true);
  });

  it('serves the channels, and main\'s own registry then stays unloaded', async () => {
    const handlers = new Map<string, (event: unknown, ...args: unknown[]) => unknown>();
    const ipc = { handle: (channel: string, fn: any) => void handlers.set(channel, fn), removeHandler: (channel: string) => void handlers.delete(channel) };
    const client = new EngineFontClient();
    const port = new FakeFontPort();
    client.connect(port);
    port.serve({ generation: 0 });
    const trusted = { id: 7 };
    const uninstall = installEngineFontService(ipc as any, { isTrustedSender: (e: any) => e.sender === trusted }, client);
    expect([...handlers.keys()].sort()).toEqual(Object.values(ENGINE_FONTS_IPC).sort());
    expect(() => installEngineFontService(ipc as any, { isTrustedSender: () => true }, client)).toThrow(/already registered/);
    const load = vi.fn();
    expect(setupEngineFonts({ ipc, ctx: { isTrustedSender: () => true }, env: { POWERMOVE_ENGINE_FONTS: '1', POWERMOVE_PM_BRIDGE: __filename }, appPath: '/app', load })).toBeNull();
    expect(load).not.toHaveBeenCalled();

    expect(handlers.get(ENGINE_FONTS_IPC.answer)!({ sender: {} }, '[]')).toBeNull();
    // The first answer goes without `known`; later ones carry the generation the window saw.
    const first = handlers.get(ENGINE_FONTS_IPC.answer)!({ sender: trusted }, '[]') as Promise<unknown>;
    port.serve({ generation: 3 });
    expect(await first).toEqual({ ok: true, value: '{"answers":[],"faces":[],"generation":3}' });
    const second = handlers.get(ENGINE_FONTS_IPC.answer)!({ sender: trusted }, '[]') as Promise<unknown>;
    expect(port.sent[0]!.json).toBe('{"queries":[],"known":3}');
    port.serve({ generation: 3 });
    await second;
    expect(handlers.get(ENGINE_FONTS_IPC.subset)!({ sender: trusted }, '../x', 0)).toEqual({ ok: false, error: 'invalid subset key' });
    const failed = handlers.get(ENGINE_FONTS_IPC.generation)!({ sender: trusted }) as Promise<unknown>;
    port.reply(port.sent[0]!.id, FONT_STATUS.error, 'boom');
    expect(await failed).toEqual({ ok: false, error: 'boom' });
    uninstall();
    expect(handlers.size).toBe(0);
  });
});
