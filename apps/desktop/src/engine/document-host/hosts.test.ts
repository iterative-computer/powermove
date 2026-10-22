/*
 * What a document host worker does without a browser: captions from speech
 * through the host's transcription service, rasterless edits under the
 * auditor and dual answers with zero fallbacks, and extension registration
 * (a local extension's effect usable by the next call; a Store extension
 * refused by name).
 */
import { existsSync, mkdirSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import { afterAll, afterEach, beforeAll, describe, expect, it } from 'vitest';
import type { AgentToolResponseEvent } from '../../shared/ipc';
import type { ExtensionRecord } from '../../shared/extensions';
import type { TranscribeProgress, TranscribeRequest } from '../../shared/transcription';
import { DocumentHostPool } from './pool';
import type { DocumentHostServices } from './protocol';
import { BRIDGE, FIXTURE, WASM, buildEngineBundle, effectSource, extensionHost, hostServices, userRecord } from './test-host';

let bundle: { script: string; dispose(): void } | null = null;
beforeAll(async () => { bundle = await buildEngineBundle(); }, 180_000);
afterAll(() => bundle?.dispose());

const pools: DocumentHostPool[] = [];
const newPool = (services: DocumentHostServices, env: Record<string, string> = {}) => {
  const pool = new DocumentHostPool({ services, newEngine: true, script: bundle!.script, env: { POWERMOVE_PM_WASM: WASM, POWERMOVE_PM_BRIDGE: BRIDGE, ...env } });
  pools.push(pool);
  return pool;
};
afterEach(async () => { await Promise.all(pools.splice(0).map((pool) => pool.shutdown())); });

let call = 0;
const revisions = new Map<string, number>();
async function ask(pool: DocumentHostPool, projectId: string, runId: string, tool: string, args: Record<string, unknown>): Promise<AgentToolResponseEvent> {
  const response = await pool.call(projectId, { runId, callId: `h${++call}`, tool, arguments: args, baseRevision: revisions.get(projectId) ?? 0 });
  if (typeof response.revision === 'number') revisions.set(projectId, response.revision);
  return response;
}
const text = (r: AgentToolResponseEvent) => JSON.stringify(r.content);

/** The fixture with a 6 s voice-over clip (audio asset in the host's media store). */
function withVoice(id: string) {
  const project = structuredClone(FIXTURE);
  project.id = id;
  project.assets = { Avoice: { id: 'Avoice', kind: 'audio', name: 'Voice.wav', storageKey: 'media:voice', size: 1024, duration: 6, mime: 'audio/wav' } };
  project.layers = [{
    id: 'Lvoice', type: 'audio', name: 'Voice', from: 0, dur: 6, color: '#7f8cff', visible: true, lock: false, solo: false,
    p: {}, d: { asset: 'Avoice', trim: 0, volume: { v: 1, kf: [], expr: null }, speed: 1 },
  }];
  return project;
}

describe.skipIf(!existsSync(WASM) || !existsSync(BRIDGE))('a document host worker without a browser', () => {
  it('generates captions through the host transcription service', async () => {
    revisions.clear();
    const progress = new Set<(p: TranscribeProgress) => void>();
    const requests: TranscribeRequest[] = [];
    const lookups: unknown[] = [];
    const host = hostServices(new Map(), {
      mediaPath: { lookup: async (request) => { lookups.push(request); return { path: '/media/voice.wav', origin: 'store' } as never; } },
      transcription: {
        status: async () => ({ available: true, activeModelId: 'word-model', models: [{ id: 'word-model', name: 'Word model', description: '', size: 1, languages: 'multi', speed: 1, accuracy: 1, recommended: true, wordTimestamps: true, state: 'ready', timing: 'word' }] }),
        transcribe: async (request) => {
          requests.push(request);
          for (const listener of progress) listener({ requestId: request.requestId!, progress: 0.5 });
          return { ok: true, transcript: { modelId: 'word-model', language: 'en', duration: 6, timing: 'word', segments: [
            { start: 0.5, end: 2.1, text: 'Hello there.', words: [{ text: 'Hello', start: 0.5, end: 1.0 }, { text: 'there.', start: 1.1, end: 2.1 }] },
            { start: 3.0, end: 4.4, text: 'Welcome back.', words: [{ text: 'Welcome', start: 3.0, end: 3.6 }, { text: 'back.', start: 3.7, end: 4.4 }] },
          ] } } as never;
        },
        cancelTranscribe: async () => {},
        onProgress: (listener) => { progress.add(listener); return () => { progress.delete(listener); }; },
      },
    });
    host.seed('Pvoice', 'Voice', withVoice('Pvoice'));
    const pool = newPool(host.services);
    const result = await ask(pool, 'Pvoice', 'run-c', 'generate_captions', { layerIds: ['Lvoice'] });
    expect(result.error ?? null).toBeNull();
    expect(result.ok).toBe(true);
    expect(requests).toHaveLength(1);
    expect(requests[0]).toMatchObject({ path: '/media/voice.wav', wordTimestamps: true });
    expect(lookups[0]).toMatchObject({ assetId: 'Avoice', projectId: 'Pvoice', storageKey: 'media:voice' });
    expect((await ask(pool, 'Pvoice', 'run-c', '__finish_run', { commit: true })).ok).toBe(true);
    const doc = host.sessions.document('Pvoice') as any;
    const captions = doc.layers.find((l: any) => l.type === 'captions');
    expect(captions).toBeTruthy();
    expect(JSON.stringify(captions)).toContain('Welcome');
    expect((await pool.stats('Pvoice')).historyDepth).toBe(1);
  }, 120_000);

  it('says no model is available when the host has no transcription service', async () => {
    revisions.clear();
    const host = hostServices();
    host.seed('Pvoice', 'Voice', withVoice('Pvoice'));
    const pool = newPool(host.services);
    const result = await ask(pool, 'Pvoice', 'run-c', 'generate_captions', { layerIds: ['Lvoice'] });
    expect(result.ok).toBe(false);
    expect(result.error).toMatch(/model/i);
  }, 60_000);

  it('runs rasterless edits under the auditor and dual answers with zero fallbacks', async () => {
    revisions.clear();
    const host = hostServices();
    host.seed('Pedit', 'Edits');
    const pool = newPool(host.services, { POWERMOVE_NATIVE_EVALUATOR: 'audit' });
    const opened = await pool.open('Pedit');
    expect(opened.engine.mode).toBe('audit');
    const run = 'run-e';
    const apply = async (commands: unknown[]) => {
      const r = await ask(pool, 'Pedit', run, 'apply_commands', { commands, label: 'edits' });
      expect(r.error ?? null).toBeNull();
      return r;
    };
    await apply([
      { type: 'add_layer', id: 'Lnull', layerType: 'null', name: 'Rig' },
      { type: 'add_layer', id: 'Lsolid', layerType: 'solid', name: 'Plate', parent: 'Lnull', content: { w: 200, h: 120 } },
      { type: 'add_layer', id: 'Lshape', layerType: 'shape', name: 'Badge' },
    ]);
    await apply([
      { type: 'set_property', target: 'Lnull', path: 'position.x', value: 120, time: 0 },
      { type: 'set_property', target: 'Lnull', path: 'position.x', value: 480, time: 2, ease: 'easeInOut' },
      { type: 'set_property', target: 'Lsolid', path: 'rotation', value: 30, time: 1 },
      { type: 'set_expression', target: 'Lsolid', path: 'opacity', expression: '50 + Math.sin(time) * 50' },
      { type: 'offset_property', target: 'Lsolid', path: 'position.y', delta: 40 },
      { type: 'set_layer', target: 'Lshape', patch: { from: 1, duration: 3 } },
      { type: 'reorder_layer', target: 'Lshape', index: 0 },
    ]);
    await apply([{ type: 'group_layers', targets: ['Lsolid', 'Lshape'], name: 'Group' }]);
    // Text edits, those that need glyph ink included (font-anchor capture).
    await apply([{ type: 'add_layer', id: 'Ltext', layerType: 'text', name: 'Title', content: { text: 'Hello' } }]);
    await apply([
      { type: 'set_content', target: 'Ltext', patch: { text: 'Hello, world' } },
      { type: 'set_content', target: 'Ltext', patch: { size: 72, weight: 500 } },
      { type: 'set_content', target: 'Ltext', patch: { tracking: 6, font: 'Helvetica Neue' } },
      { type: 'set_property', target: 'Ltext', path: 'anchor.x', value: 40, time: 0 },
    ]);
    expect((await ask(pool, 'Pedit', run, 'get_project_state', {})).ok).toBe(true);
    expect((await ask(pool, 'Pedit', run, 'check_project', {})).ok).toBe(true);
    expect((await ask(pool, 'Pedit', run, '__finish_run', { commit: true })).ok).toBe(true);
    // The auditor checks at task boundaries every 100 ms.
    await new Promise((resolve) => setTimeout(resolve, 400));
    const stats = await pool.stats('Pedit');
    expect(stats.fallbacks).toEqual({});
    expect(stats.calls).toBeGreaterThan(0);
    expect(stats.auditor).not.toBeNull();
    expect(stats.auditor!.audits).toBeGreaterThan(0);
    expect(stats.auditor!.failures).toBe(0);
    expect(stats.dual).not.toBeNull();
    expect(stats.dual!.compared).toBeGreaterThan(0);
    expect(stats.dual!.mismatchedOps).toEqual({});
    expect(stats.dual!.mismatched).toBe(0);
  }, 120_000);

  it('answers 3D geometry natively: state of selected 3D objects and group placement, with no three.js', async () => {
    revisions.clear();
    const host = hostServices();
    host.seed('P3d', '3D');
    const pool = newPool(host.services, { POWERMOVE_NATIVE_EVALUATOR: 'audit' });
    await pool.open('P3d');
    const object = (id: string, x: number, primitive: string) => ({
      type: 'add_layer', id, layerType: 'extension', name: id,
      content: { definition: 'powermove.3d.object', data: { object: { source: { primitive } } } },
      properties: { 'position.x': x, 'rotation.y': 30 },
    });
    const added = await ask(pool, 'P3d', 'run-3d', 'apply_commands', { commands: [object('Obox', -1.5, 'box'), object('Osphere', 2, 'sphere'), object('Ocone', 0, 'cone'),
      { type: 'add_layer', id: 'Lplate', layerType: 'solid', name: 'Plate', content: { w: 200, h: 120 } }], label: 'objects' });
    expect(added.error ?? null).toBeNull();
    const run = 'run-3d';
    expect((await ask(pool, 'P3d', run, 'select_layers', { layerIds: ['Obox', 'Osphere'] })).error ?? null).toBeNull();
    const state = await ask(pool, 'P3d', run, 'get_project_state', {});
    expect(state.error ?? null).toBeNull();
    expect(state.ok).toBe(true);
    const grouped = await ask(pool, 'P3d', run, 'apply_commands', { commands: [
      // A model group (3D members only) and a mixed group (3D bounds in the editor's view).
      { type: 'group_layers', targets: ['Obox', 'Osphere'], name: 'Model' },
      { type: 'group_layers', targets: ['Ocone', 'Lplate'], name: 'Mixed' },
    ], label: 'group' });
    expect(grouped.error ?? null).toBeNull();
    expect((await ask(pool, 'P3d', run, 'get_3d_scene', {})).ok).toBe(true);
    const stats = await pool.stats('P3d');
    expect(stats.failures).toEqual([]);
    expect(stats.fallbacks).toEqual({});
    expect(stats.dual!.mismatched).toBe(0);
    // The geometry questions were asked, and answered by pm-wasm.
    expect(stats.dual!.comparedOps['edit.groupPlacement']).toBeGreaterThan(0);
    expect(stats.calls).toBeGreaterThan(0);
  }, 120_000);

  it('answers nested and replacement-time scopes natively in a required-engine worker', async () => {
    const project = structuredClone(FIXTURE);
    project.comps = { outer: { id: 'outer', layers: [], comps: { same: { id: 'same', fps: 24, layers: [{ id: 'Linner', type: 'solid', from: 0, dur: 10, p: { opacity: { v: 73, kf: [], expr: null } }, d: { w: 100, h: 100 } }] } } } };
    const record = userRecord('scope-check');
    const ext = extensionHost([record], {
      [record.bundleUrl!]: `export default function activate(api) {
        const pm = api.host.pm;
        const inner = pm.proj.comps.outer.comps.same;
        pm.scope = [inner];
        const check = () => { if (pm.EngineDoc.api.ev(inner.layers[0], 'opacity', 0) !== 73) throw new Error('wrong nested answer'); };
        try {
          check();
          const comps = pm.proj.comps;
          pm.proj.comps = {}; pm.doc.touched([['comps']]);
          check();
          pm.proj.comps = comps; pm.doc.touched([['comps']]);
        } finally { pm.scope = []; }
      }`,
    });
    const host = hostServices(new Map(), { extensions: ext.service });
    host.seed('Pscope', 'Scopes', project);
    const pool = newPool(host.services);
    await pool.open('Pscope');
    const stats = await pool.stats('Pscope');
    expect(stats.extensions.failed).toEqual([]);
    expect(stats.calls).toBeGreaterThanOrEqual(2);
    expect(stats.fallbacks).toEqual({});
    expect(stats.failures).toEqual([]);
  }, 60_000);

  it('records font-anchor bounds natively: text edits that need glyph ink have no fallback', async () => {
    // A size, font, weight, tracking or box edit on a text layer records its
    // ink bounds (font-anchor.ts captureFontAnchor). The worker loads whole
    // faces (pm-bridge FontRegistry.whole), so the kernel measures the ink
    // natively; nothing falls back to the legacy edit.
    revisions.clear();
    const host = hostServices();
    host.seed('Ptext', 'Text');
    const pool = newPool(host.services);
    await ask(pool, 'Ptext', 'run-t', 'apply_commands', { commands: [{ type: 'add_layer', id: 'Ltext', layerType: 'text', name: 'Title', content: { text: 'Hello' } }], label: 'text' });
    for (const patch of [{ size: 96 }, { weight: 400 }, { tracking: 4 }, { font: 'Helvetica Neue' }, { boxWidth: 600 }]) {
      const edited = await ask(pool, 'Ptext', 'run-t', 'apply_commands', { commands: [{ type: 'set_content', target: 'Ltext', patch }], label: 'type' });
      expect(edited.error ?? null, JSON.stringify(patch)).toBeNull();
    }
    const stats = await pool.stats('Ptext');
    expect(stats.fallbacks).toEqual({});
    expect(stats.failures).toEqual([]);
    const layer = (host.sessions.document('Ptext') as any).layers.find((l: any) => l.id === 'Ltext');
    expect(layer.d.fontAnchorBounds).toMatchObject({ x0: expect.any(Number), y0: expect.any(Number), x1: expect.any(Number), y1: expect.any(Number) });
    expect(layer.d.fontAnchorBounds.x1).toBeGreaterThan(layer.d.fontAnchorBounds.x0);
  }, 60_000);

  it("loads the user's extensions as the editor does: overrides, forks, dependencies, disabled records, live changes", async () => {
    revisions.clear();
    const host = hostServices();
    host.seed('Pext', 'Extensions');
    const ext = extensionHost([
      // A user effect pack.
      userRecord('user-glow'),
      // A fork of effects-basic: it replaces the built-in, with its own blur.
      userRecord('my-effects', { replaces: ['effects-basic'] }),
      // Needs an extension that is not installed: skipped, as in the editor.
      userRecord('needs-missing', { dependsOn: ['not-installed'] }),
      // Turned off in Mods.
      userRecord('turned-off', {}, { enabled: false, health: { state: 'disabled' } }),
    ], {
      'app://powermove/ext/user-glow/bundle.js?v=h9': `export default function activate(api) { ${effectSource('user-glow.glow', 0.25)} }`,
      'app://powermove/ext/my-effects/bundle.js?v=h10': `export default function activate(api) { ${effectSource('blur', 42)} }`,
      'app://powermove/ext/needs-missing/bundle.js?v=h13': `export default function activate(api) { ${effectSource('needs-missing.fx')} }`,
      'app://powermove/ext/turned-off/bundle.js?v=h10': `export default function activate(api) { ${effectSource('turned-off.fx')} }`,
    });
    host.services.extensions = ext.service;
    const pool = newPool(host.services);
    await pool.open('Pext');
    const stats = await pool.stats('Pext');
    expect(stats.extensions.active).toEqual(expect.arrayContaining(['user-glow', 'my-effects', 'timeline', 'inspector']));
    expect(stats.extensions.active).not.toContain('effects-basic');
    expect(stats.extensions.active).not.toContain('needs-missing');
    expect(stats.extensions.active).not.toContain('turned-off');
    // Bundles of records that do not load are never fetched.
    expect(ext.fetched).not.toContain('app://powermove/ext/turned-off/bundle.js?v=h10');

    const effects = async (commands: unknown[]) => {
      const r = await ask(pool, 'Pext', 'run-x', 'apply_commands', { commands, label: 'fx' });
      return r;
    };
    const ok = await effects([
      { type: 'add_layer', id: 'Lplate', layerType: 'solid', name: 'Plate' },
      { type: 'add_effect', target: 'Lplate', effect: 'user-glow.glow' },
      { type: 'add_effect', target: 'Lplate', effect: 'blur' },
    ]);
    expect(ok.error ?? null).toBeNull();
    expect((await effects([{ type: 'add_effect', target: 'Lplate', effect: 'invert' }])).error).toMatch(/Unknown effect: invert/);
    expect((await effects([{ type: 'add_effect', target: 'Lplate', effect: 'turned-off.fx' }])).error).toMatch(/Unknown effect/);
    await ask(pool, 'Pext', 'run-x', '__finish_run', { commit: true });
    const plate = (host.sessions.document('Pext') as any).layers.find((l: any) => l.id === 'Lplate');
    expect(plate.fx.map((f: any) => [f.type, f.p.amount.v])).toEqual([['user-glow.glow', 0.25], ['blur', 42]]);

    // The user turns the fork off in Mods: the host's list changes and the built-in comes back.
    ext.records = ext.records.map((r) => (r.id === 'my-effects' ? { ...r, enabled: false, health: { state: 'disabled' } } : r));
    ext.changed({ ids: ['my-effects'], reason: 'disable' });
    await new Promise((resolve) => setTimeout(resolve, 500));
    const after = await pool.stats('Pext');
    expect(after.extensions.active).toContain('effects-basic');
    expect(after.extensions.active).not.toContain('my-effects');
    expect((await effects([{ type: 'add_effect', target: 'Lplate', effect: 'invert' }])).error ?? null).toBeNull();
  }, 120_000);

  it('registers and hot-reloads an extension through the loader, so the next call uses it', async () => {
    revisions.clear();
    const host = hostServices();
    host.seed('Pext', 'Extensions');
    const ext = extensionHost();
    host.services.extensions = ext.service;
    const pool = newPool(host.services);
    const dir = path.join(path.dirname(bundle!.script), 'ext-glow');
    mkdirSync(dir, { recursive: true });
    const entry = path.join(dir, 'index.mjs');
    const record = (enabled = true): ExtensionRecord => ({
      ...userRecord('test-glow'), bundleUrl: pathToFileURL(entry).href, enabled, ...(enabled ? {} : { health: { state: 'disabled' } }),
    } as ExtensionRecord);
    const add = () => ask(pool, 'Pext', 'run-x', 'apply_commands', { commands: [{ type: 'add_layer', id: 'Lplate', layerType: 'solid', name: 'Plate' }, { type: 'add_effect', target: 'Lplate', effect: 'test.glow' }], label: 'glow' });
    expect((await add()).error).toMatch(/Unknown effect: test\.glow/);
    writeFileSync(entry, `export default function activate(api) { ${effectSource('test.glow', 0.5)} }\n`);
    // A disabled record stays off (planLoad), as in the editor.
    expect(await pool.registerExtension('Pext', record(false))).toMatchObject({ extension: 'test-glow', active: false });
    expect((await add()).error).toMatch(/Unknown effect: test\.glow/);
    const registered = await pool.registerExtension('Pext', record());
    expect(registered).toMatchObject({ extension: 'test-glow', active: true });
    expect(registered.definitionsGeneration).toBeGreaterThan(0);
    expect((await add()).error ?? null).toBeNull();
    // A rebuilt bundle: the next registration runs the new code.
    writeFileSync(entry, `export default function activate(api) { ${effectSource('test.glow', 0.9)} }\n`);
    await pool.registerExtension('Pext', record());
    await ask(pool, 'Pext', 'run-x', 'apply_commands', { commands: [{ type: 'add_effect', target: 'Lplate', effect: 'test.glow' }], label: 'glow 2' });
    await ask(pool, 'Pext', 'run-x', '__finish_run', { commit: true });
    const plate = (host.sessions.document('Pext') as any).layers.find((l: any) => l.id === 'Lplate');
    expect(plate.fx.map((f: any) => f.p.amount.v)).toEqual([0.5, 0.9]);
    expect((await pool.stats('Pext')).extensions.active).toContain('test-glow');
    expect(text(await ask(pool, 'Pext', 'run-y', 'get_project_state', {}))).toBeTruthy();
  }, 120_000);

  it('runs Store extensions in a contained process: their contributions work, the host stays out of reach', async () => {
    revisions.clear();
    const host = hostServices();
    host.seed('Pstore', 'Store');
    const probe = `export default async function activate(api) {
  const results = {};
  try { await import('node:fs'); results.fs = 'escaped'; } catch { results.fs = 'refused'; }
  try { await import('node:child_process'); results.exec = 'escaped'; } catch { results.exec = 'refused'; }
  try { await fetch('https://127.0.0.1:9/'); results.net = 'escaped'; } catch (error) { results.net = error instanceof TypeError && error.message === 'Failed to fetch' ? 'refused' : 'error'; }
  results.proc = typeof process === 'undefined' || (process.getBuiltinModule === undefined && process.binding === undefined && process.dlopen === undefined) ? 'refused' : 'escaped';
  results.env = typeof process === 'undefined' || Object.keys(process.env).every((k) => ['PATH', 'LANG', 'TZ'].includes(k)) ? 'clean' : 'leaked';
  const register = (id, def = 0.5) => api.effects.register({ id, label: id, group: 'Stylize', params: [{ k: 'amount', label: 'Amount', def, min: 0, max: 100 }],
    frag: '#version 300 es\\nprecision highp float;\\nin vec2 v_st;\\nuniform sampler2D u_tex;\\nout vec4 o;\\nvoid main(){ o = texture(u_tex, v_st); }' });
  for (const [k, v] of Object.entries(results)) await register(k + '-' + v);
  await register('glow', 3);
}
`;
    const ext = extensionHost([userRecord('store-probe', {}, { trust: 'store', scope: 'user' } as never)], {
      'app://powermove/ext/store-probe/bundle.js?v=h11': probe,
    });
    host.services.extensions = ext.service;
    const pool = newPool(host.services);
    await pool.open('Pstore');
    const stats = await pool.stats('Pstore');
    if (stats.extensions.failed.length) console.log(host.log.filter((l) => /store-probe|contained/.test(l)).join('\n'));
    expect(stats.extensions.failed).toEqual([]);
    expect(stats.extensions.active).toContain('store-probe');
    const add = (effect: string) => ask(pool, 'Pstore', 'run-s', 'apply_commands', { commands: [{ type: 'add_effect', target: 'Lplate', effect }], label: effect });
    expect((await ask(pool, 'Pstore', 'run-s', 'apply_commands', { commands: [{ type: 'add_layer', id: 'Lplate', layerType: 'solid', name: 'Plate' }], label: 'plate' })).ok).toBe(true);
    // Its ids are namespaced at the sandbox boundary, as in the editor.
    expect((await add('store-probe.glow')).error ?? null).toBeNull();
    for (const outcome of ['fs-refused', 'exec-refused', 'net-refused', 'proc-refused', 'env-clean']) {
      expect((await add(`store-probe.${outcome}`)).error ?? null, outcome).toBeNull();
    }
    await ask(pool, 'Pstore', 'run-s', '__finish_run', { commit: true });
    const plate = (host.sessions.document('Pstore') as any).layers.find((l: any) => l.id === 'Lplate');
    expect(plate.fx[0]).toMatchObject({ type: 'store-probe.glow' });
    expect(plate.fx[0].p.amount.v).toBe(3);
    // Closing the project stops its contained process.
    await pool.close('Pstore');
  }, 120_000);
});
