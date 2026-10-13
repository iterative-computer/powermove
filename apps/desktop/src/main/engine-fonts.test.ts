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
  registerEngineFontsIpc({ handle: (channel: string, fn: any) => void handlers.set(channel, fn) } as any, { isTrustedSenderContents: s => s === trusted }, registry);
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
    expect(setupEngineFonts({ ipc: { handle }, ctx: { isTrustedSenderContents: () => true }, env: {}, appPath: '/app', load })).toBeNull();
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
    const ctx = { isTrustedSenderContents: () => true };
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
    expect(setupEngineFonts({ ipc: { handle }, ctx: { isTrustedSenderContents: () => true }, env, appPath: '/app', load: () => ({ FontRegistry }) })).toBe(registry);
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

describe('the preload channels', () => {
  it('name the registry channels main registers', async () => {
    const { IPC } = await import('../shared/ipc');
    expect(IPC.engineFontsAnswer).toBe(ENGINE_FONTS_IPC.answer);
    expect(IPC.engineFontsSubset).toBe(ENGINE_FONTS_IPC.subset);
  });
});
