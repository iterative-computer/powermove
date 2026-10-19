import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

import type { DefinitionsUpdate } from '../../../shared/engine-protocol';
import { install as installShaders } from '../legacy/gl/shaders';
import { install as installTransitions } from '../legacy/gl/transitions';
import { EFFECTS } from '../../../extensions/effects-basic/effects';
import { BUILTIN_TRANSITIONS } from '../../../extensions/transitions-basic/transitions';
import type { EffectDefinition, ExtensionLayerDefinition, TransitionDefinition } from './api';
import {
  DefinitionPublisher, SessionDefinitions, effectSnapshot, layerSnapshot, normalizedColor, originOf, paramSpec,
  transitionSnapshot, wrappedGlsl, type Preambles
} from './definition-snapshots';
import { EngineShaderMeta, describe as describeError, hashStr, shaderLayerKey } from './engine-definitions';

/** UIState's shader meta store (legacy/core/ui-state.ts), recording each patch. */
function uiState(meta: any[]) {
  const store = new Map<string, any>();
  return {
    setShaderMeta: (layer: any, patch: any) => { meta.push([layer.id, patch]); store.set(layer.id, { ...store.get(layer.id), ...patch }); },
    getShaderMeta: (layer: any) => store.get(layer.id) ?? null
  };
}
import { createKernel } from './registries';

/** The real preambles, installed as the app installs them. */
function realPreambles(): Preambles {
  const PM: any = { uid: () => 'fx', P: (v: unknown) => ({ v }) };
  installShaders(PM);
  installTransitions(PM);
  return { effect: PM.GLSL_PRE, transition: PM.TRANSITION_PRE, layer: PM.SHADER_HEADER };
}

const PRE: Preambles = { effect: '#version 300 es\nprecision highp float;\nuniform float u_prog;\nout vec4 o;', transition: '#version 300 es\nprecision highp float;\nout vec4 o;', layer: '#version 300 es\nprecision highp float;\nout vec4 fragColor;' };

const effect = (over: Partial<EffectDefinition> = {}): EffectDefinition => ({
  id: 'wave', label: 'Wave', group: 'Distort',
  params: [{ k: 'amount', label: 'Amount', def: 1, min: 0, max: 10, unit: 'px' }, { k: 'tint', label: 'Tint', def: '#F80', type: 'color' }, { k: 'on', label: 'On', def: true, type: 'toggle', advanced: true }],
  frag: '  o = src(v_st) * u_amount; // é',
  ...over
});

describe('snapshots', () => {
  it('wraps an effect body with its line offset and lengths', () => {
    const s = effectSnapshot(effect(), 'local', PRE);
    const lines = s.glsl!.source.split('\n');
    expect(lines[s.glsl!.bodyLineOffset]).toBe('  o = src(v_st) * u_amount; // é');
    expect(s.glsl!.bodyUtf16Len).toBe('  o = src(v_st) * u_amount; // é'.length);
    expect(s.glsl!.bodyUtf8Len).toBe(new TextEncoder().encode('  o = src(v_st) * u_amount; // é').length);
    expect(s.glsl!.source).toContain('#define u_p0 u_amount');
    expect(s.glsl!.source).toContain('uniform int u_pass;');
    expect(s.params).toEqual([
      { type: 'number', key: 'amount', default: 1, min: 0, max: 10, step: null },
      { type: 'color', key: 'tint', default: '#ff8800' },
      { type: 'toggle', key: 'on', default: true }
    ]);
    expect(s.flags).toEqual({ passes: 1, keepOrig: false, backdrop: false, rawShader: false, viewportSafe: false, viewportPadding: [], clamp: false });
    expect(JSON.stringify(s)).not.toMatch(/"label"|"unit"|"advanced"|"group"/);
  });

  it('keeps raw shaders verbatim, and the flags', () => {
    const frag = '#version 300 es\nprecision highp float;\nout vec4 o;\nvoid main(){ o = vec4(1.0); }';
    const s = effectSnapshot(effect({ frag, rawShader: true, passes: 3, keepOrig: true, backdrop: true, viewportSafe: true, viewportPadding: ['amount', 'tint', 'gone'], clamp: true } as any), 'store', PRE);
    expect(s.glsl).toEqual({ source: frag, bodyLineOffset: 0, bodyUtf16Len: frag.length, bodyUtf8Len: frag.length });
    expect(s.flags).toEqual({ passes: 3, keepOrig: true, backdrop: true, rawShader: true, viewportSafe: true, viewportPadding: ['amount'], clamp: true });
    expect(s.origin).toBe('store');
  });

  it('snapshots transitions and every layer renderer', () => {
    const t: TransitionDefinition = { id: 'fade', label: 'Fade', group: 'Basic', params: [], frag: 'o = mix(texture(u_from, v_st), texture(u_to, v_st), u_prog);' };
    const ts = transitionSnapshot(t, 'builtIn', PRE);
    expect(ts.glsl!.source.split('\n')[ts.glsl!.bodyLineOffset]).toBe(t.frag);
    expect(ts.flags.passes).toBe(1);
    const layer = (renderer: ExtensionLayerDefinition['renderer']): ExtensionLayerDefinition => ({ id: 'l', label: 'L', version: 3, params: [], renderer });
    const fragment = layerSnapshot(layer({ kind: 'fragment', fragment: 'void main(){ fragColor = vec4(1.0); }' }), 'local', PRE);
    expect(fragment.renderer).toEqual({ kind: 'fragment', fragment: 'void main(){ fragColor = vec4(1.0); }' });
    expect(fragment.glsl!.source.split('\n')[fragment.glsl!.bodyLineOffset]).toBe('void main(){ fragColor = vec4(1.0); }');
    expect(fragment.version).toBe(3);
    expect(layerSnapshot(layer({ kind: 'mesh', assetField: 'model' }), 'local', PRE)).toMatchObject({ renderer: { kind: 'mesh', assetField: 'model' }, glsl: null });
    expect(layerSnapshot(layer({ kind: 'layer3d', role: 'light' }), 'fork', PRE)).toMatchObject({ renderer: { kind: 'layer3d', role: 'light' }, origin: 'fork' });
  });

  it('reads colors as hex2rgb does and makes number ranges well-formed', () => {
    expect(normalizedColor('#1B2A4A')).toBe('#1b2a4a');
    expect(normalizedColor('#f08c')).toBe('#ff0088');
    expect(normalizedColor('zz8000')).toBe('#008000');
    expect(normalizedColor(undefined)).toBe('#000000');
    expect(paramSpec({ k: 'a', label: 'A', def: 5, min: 10, max: 0, step: -1 } as any)).toEqual({ type: 'number', key: 'a', default: 5, min: 0, max: 10, step: null });
    expect(paramSpec({ k: 'a', label: 'A', def: NaN, min: NaN, max: Infinity } as any)).toEqual({ type: 'number', key: 'a', default: 0, min: 0, max: 0, step: null });
    expect(paramSpec({ k: 'mode', label: 'Mode', def: 2, type: 'select' } as any)).toMatchObject({ type: 'number', default: 2 });
    expect(() => wrappedGlsl('abc', 'x')).toThrow();
  });

  it('takes the origin from the extension record', () => {
    expect(originOf({ scope: 'builtin', trust: 'builtin' })).toBe('builtIn');
    expect(originOf({ scope: 'user', trust: 'local', manifest: { forkedFrom: 'effects-basic@1.0.2', replaces: ['effects-basic'] } })).toBe('fork');
    expect(originOf({ scope: 'user', trust: 'store' })).toBe('store');
    expect(originOf({ scope: 'user', trust: 'store-trusted' })).toBe('local');
    expect(originOf(undefined)).toBe('local');
  });

  it('matches the built-in snapshots pm-render embeds', () => {
    const preambles = realPreambles();
    const definitions = [
      ...EFFECTS.map((d) => effectSnapshot(d, 'builtIn', preambles)),
      ...BUILTIN_TRANSITIONS.map((d) => transitionSnapshot(d, 'builtIn', preambles))
    ];
    const path = join(__dirname, '../../../../../../engine/crates/pm-render/src/effects/builtins.json');
    const embedded = JSON.parse(readFileSync(path, 'utf8'));
    // Run `bun engine/crates/pm-render/tools/effects/builtins.ts` after changing a built-in.
    expect(embedded.definitions).toEqual(JSON.parse(JSON.stringify(definitions)));
    expect(definitions).toHaveLength(20);
  });
});

describe('the publisher', () => {
  const tick = () => new Promise<void>((resolve) => queueMicrotask(resolve));

  it('follows the winning registration through register, override, dispose, and reload', async () => {
    const kernel = createKernel();
    const publisher = new DefinitionPublisher(kernel, { origin: (owner) => (owner === 'builtin' ? 'builtIn' : 'local'), preambles: () => PRE });
    const changes: any[] = [];
    publisher.subscribe((c) => changes.push(c));
    const base = kernel.registerEffect('builtin', effect());
    await tick();
    expect(changes.pop()).toMatchObject({ upserts: [{ id: 'wave', origin: 'builtIn' }], removals: [] });
    const fork = kernel.registerEffect('fork', effect({ frag: 'o = vec4(1.0);' }));
    await tick();
    const forked = changes.pop();
    expect(forked.upserts[0].origin).toBe('local');
    expect(forked.upserts[0].glsl.source).toContain('o = vec4(1.0);');
    fork.dispose();
    await tick();
    expect(changes.pop().upserts[0].origin).toBe('builtIn');
    // A reload: dispose and register again inside a batch → one change.
    const batch = kernel.effects.batchChanges();
    base.dispose();
    const again = kernel.registerEffect('builtin', effect({ frag: 'o = vec4(0.5);' }));
    batch.dispose();
    await tick();
    expect(changes).toHaveLength(1);
    expect(changes.pop().upserts[0].glsl.source).toContain('o = vec4(0.5);');
    again.dispose();
    await tick();
    expect(changes.pop()).toEqual({ upserts: [], removals: [{ id: 'wave', kind: 'effect' }] });
    // Several changes in one task are one change.
    kernel.registerEffect('a', effect({ id: 'one' }));
    kernel.registerTransition('a', { id: 'two', label: 'Two', params: [], frag: 'o = vec4(u_prog);' } as TransitionDefinition);
    await tick();
    expect(changes).toHaveLength(1);
    expect(changes[0].upserts.map((u: any) => `${u.kind}:${u.id}`).sort()).toEqual(['effect:one', 'transition:two']);
  });

  it('numbers a session\'s updates and starts again with a full one after a refusal', async () => {
    const kernel = createKernel();
    kernel.registerEffect('a', effect({ id: 'one' }));
    const publisher = new DefinitionPublisher(kernel, { origin: () => 'local', preambles: () => PRE });
    const sent: DefinitionsUpdate[] = [];
    let accept = true;
    const session = new SessionDefinitions(publisher, (u) => { sent.push(u); return accept; });
    expect(sent[0]).toMatchObject({ generation: 1, full: true, removals: [] });
    expect(sent[0]!.upserts.map((u) => u.id)).toEqual(['one']);
    kernel.registerEffect('a', effect({ id: 'two' }));
    await tick();
    expect(sent[1]).toMatchObject({ generation: 2, full: false, upserts: [{ id: 'two' }] });
    accept = false;
    kernel.registerEffect('a', effect({ id: 'three' }));
    await tick();
    expect(sent[2]).toMatchObject({ generation: 3, full: false });
    accept = true;
    kernel.registerEffect('a', effect({ id: 'four' }));
    await tick();
    expect(sent[3]).toMatchObject({ generation: 4, full: true });
    expect(sent[3]!.upserts.map((u) => u.id).sort()).toEqual(['four', 'one', 'three', 'two']);
    session.dispose();
    kernel.registerEffect('a', effect({ id: 'five' }));
    await tick();
    expect(sent).toHaveLength(4);
  });

  it('leaves out a definition it cannot snapshot and reports it', async () => {
    const kernel = createKernel();
    const errors: string[] = [];
    const publisher = new DefinitionPublisher(kernel, { origin: () => 'local', preambles: () => { throw new Error('no preamble'); }, onError: (kind, id) => errors.push(`${kind}:${id}`) });
    kernel.registerEffect('a', effect());
    expect(publisher.all()).toEqual([]);
    expect(errors).toEqual(['effect:wave']);
  });
});

describe('engine shader meta', () => {
  const diagnostics = { kind: 'shaderCompile' as const, diagnostics: [{ line: 3, column: null, message: "'nope' : undeclared identifier" }, { line: 0, column: null, message: 'in the preamble' }] };

  it('fills compile errors and layer meta from engine reports', () => {
    const meta: any[] = [];
    const layers = [{ id: 'L1', type: 'extension', d: { definition: 'gen.glow' } }, { id: 'L2', type: 'shader', d: {} }];
    const PM: any = { proj: { layers, comps: {} }, UIState: uiState(meta) };
    const m = new EngineShaderMeta(PM);
    m.applied({ generation: 2, failures: [{ code: 'shaderCompile', scope: { kind: 'definition', definition: 'gen.glow' }, message: 'The shader of gen.glow does not compile.', retry: { kind: 'afterUserAction' }, detail: diagnostics }] });
    expect(m.compileError('fx:gen.glow')).toBe("The shader of gen.glow does not compile.\nline 3: 'nope' : undeclared identifier\n(generated code): in the preamble");
    expect(meta).toEqual([['L1', { shaderKey: 'extension:gen.glow', definition: 'gen.glow', missing: false }]]);
    m.pushed({ generation: 3, full: false, upserts: [], removals: [{ id: 'gen.glow', kind: 'effect' }] });
    expect(m.compileError('fx:gen.glow')).toBeNull();
    // Layer reports apply at their revision; older ones are ignored.
    m.failure({ revision: 7, error: { code: 'shaderCompile', scope: { kind: 'layer', layer: 'L2' }, message: 'Shader layer L2 does not compile.', retry: { kind: 'never' }, detail: diagnostics } });
    // A shader layer's program has today's key (`sh:<id>:<hashStr(code)>`).
    expect(m.compileError('engine:L2')).toContain('line 3');
    expect(m.compileError('sh:L2:5381')).toContain('line 3');
    expect(meta.at(-1)).toEqual(['L2', { shaderKey: 'sh:L2:5381', definition: null, missing: false }]);
    m.failure({ revision: 5, error: { code: 'definitionMissing', scope: { kind: 'layer', layer: 'L2' }, message: 'old', retry: { kind: 'never' }, detail: null } });
    expect(meta).toHaveLength(2);
    m.failure({ revision: 8, error: { code: 'watchdogTimeout', scope: { kind: 'effect', layer: 'L2', effect: 'f1', definition: 'blur' }, message: 'blur took 300 ms.', retry: { kind: 'afterUserAction' }, detail: null } });
    expect(m.compileError('fx:blur')).toBe('blur took 300 ms.');
    expect(describeError({ code: 'internal', scope: { kind: 'connection' }, message: 'x', retry: { kind: 'never' }, detail: null })).toBe('x');
  });

  it('names every drawn shader layer\'s program as today\'s compositor did, and its error under that key', () => {
    const meta: any[] = [];
    const code = 'void main(){ fragColor = vec4(1.); }';
    const layers = [{ id: 'S', type: 'shader', d: { code } }, { id: 'T', type: 'text', d: { text: 'x' } }];
    const PM: any = { proj: { layers, comps: {} }, UIState: uiState(meta) };
    const m = new EngineShaderMeta(PM);
    const key = `sh:S:${hashStr(code)}`;
    expect(shaderLayerKey(layers[0]!)).toBe(key);
    // legacy/gl/compositor.ts hashStr (djb2, 32-bit signed).
    expect(hashStr('void main(){}')).toBe(-760748891);
    m.frame(1);
    expect(PM.UIState.getShaderMeta(layers[0])).toEqual({ shaderKey: key });
    expect(m.compileError(key)).toBeNull();
    // Unchanged: not set again.
    const count = meta.length;
    m.frame(2);
    expect(meta).toHaveLength(count);
    m.failure({ revision: 3, error: { code: 'shaderCompile', scope: { kind: 'layer', layer: 'S' }, message: 'Shader layer S does not compile.', retry: { kind: 'afterUserAction' }, detail: diagnostics } });
    expect(m.compileError(key)).toContain('line 3');
    // The code changed: a new key; the old error goes when the frame stops repeating it.
    layers[0]!.d.code = code + ' ';
    m.frame(4);
    expect(PM.UIState.getShaderMeta(layers[0]).shaderKey).toBe(`sh:S:${hashStr(code + ' ')}`);
    expect(m.compileError(key)).toBeNull();
  });

  it('holds layer reports for their revision and clears the ones a later frame did not repeat', () => {
    const meta: any[] = [];
    const layers = [{ id: 'S', type: 'shader', d: {} }, { id: 'X', type: 'extension', d: { definition: 'gen.orb' } }, { id: 'Y', type: 'extension', d: { definition: 'gen.bars' } }];
    const PM: any = { proj: { layers, comps: {} }, UIState: uiState(meta) };
    const m = new EngineShaderMeta(PM);
    const compile = (revision: number) => m.failure({ revision, error: { code: 'shaderCompile', scope: { kind: 'layer', layer: 'S' }, message: 'Shader layer S does not compile.', retry: { kind: 'afterUserAction' }, detail: diagnostics } });
    const missing = (revision: number, layer: string) => m.failure({ revision, error: { code: 'definitionMissing', scope: { kind: 'layer', layer }, message: `Extension layer ${layer} is missing.`, retry: { kind: 'never' }, detail: null } });
    compile(3);
    missing(3, 'X');
    missing(3, 'Y');
    m.frame(3);
    // A frame at the reports' own revision keeps them.
    expect(m.compileError('engine:S')).toContain('line 3');
    expect(meta.filter(([, p]) => p.missing === false && p.missingAsset === null)).toEqual([]);
    // Revision 4 repeats S and X before its frame; Y was not repeated.
    compile(4);
    missing(4, 'X');
    m.frame(4);
    expect(m.compileError('engine:S')).toContain('line 3');
    expect(meta.at(-1)).toEqual(['Y', { missing: false, missingAsset: null }]);
    // Revision 5 repeats nothing: the code compiles again, X is cleared.
    m.frame(5);
    expect(m.compileError('engine:S')).toBeNull();
    expect(meta.at(-1)).toEqual(['X', { missing: false, missingAsset: null }]);
    // A pushed definition clears the missing state of the layers drawn from it.
    missing(6, 'Y');
    const before = meta.length;
    m.pushed({ generation: 9, full: false, upserts: [{ id: 'gen.bars' } as any], removals: [] });
    expect(meta.slice(before)).toEqual([['Y', { missing: false, missingAsset: null }]]);
    // A report of another kind replaces the one in force.
    missing(7, 'S');
    compile(8);
    expect(meta.at(-2)).toEqual(['S', { missing: false, missingAsset: null }]);
    expect(meta.at(-1)).toEqual(['S', { shaderKey: 'sh:S:5381', definition: null, missing: false }]);
  });

  it('shows a shader layer the fallback runner draws as not headless-renderable until a frame stops repeating it', () => {
    const meta: any[] = [];
    const PM: any = { proj: { layers: [{ id: 'C', type: 'shader', d: {} }], comps: {} }, UIState: uiState(meta) };
    const m = new EngineShaderMeta(PM);
    const message = 'Shader layer C is drawn by the WebGL fallback, so headless exports cannot render it (the native pipeline stops at bind: samplerCube uEnv is not a float sampler2D).';
    m.failure({ revision: 2, error: { code: 'notHeadlessRenderable', scope: { kind: 'layer', layer: 'C' }, message, retry: { kind: 'afterUserAction' }, detail: { kind: 'shaderCompile', diagnostics: [] } } });
    m.frame(2);
    expect(m.compileError('engine:C')).toBe(message);
    expect(meta).toEqual([['C', { shaderKey: 'engine:C', definition: null, missing: false }]]);
    // The code changed and the native pipeline draws it: not repeated.
    m.frame(3);
    expect(m.compileError('engine:C')).toBeNull();
  });
});
