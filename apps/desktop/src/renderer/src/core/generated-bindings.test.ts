import { describe, expect, it, vi } from 'vitest';
import type { GeneratedButtonControl, GeneratedControl } from './types/workspace';
import {
  currentParam,
  ensureParam,
  formatPreview,
  generatedButtonDisabled,
  resolveButtonAction,
  sceneParameterCommand,
  sourceBinding,
  stripManifestCommands
} from './generated-bindings';

const control = (value: Record<string, unknown>): GeneratedControl => ({
  type: 'slider', label: 'Value', stateKey: '', param: 'Value', min: 0, max: 100, def: 0, step: 1,
  ...value
} as GeneratedControl);

function fakePM() {
  const layers: Record<string, any> = {
    L1: { id: 'L1', name: 'First', on: true, lock: false, dur: 4, mblur: true, from: 1, d: { text: 'One' }, p: { opacity: { v: 20 } } },
    L2: { id: 'L2', name: 'Second', on: false, lock: true, dur: 8, mblur: false, from: 2, d: { text: 'Two' }, p: { opacity: { v: 80 } } }
  };
  const PM: Record<string, any> = {
    time: 2.5,
    sel: { layers: ['L1'], keys: [] },
    proj: {
      name: 'Demo', w: 1920, h: 1080, fps: 20, dur: 10, shutter: .25, bg: '#101010',
      work: [2, 8], backgroundFill: { type: 'solid', angle: 0, stops: [{ id: 'a', color: '#101010', position: 0 }] },
      params: {}, layers: Object.values(layers)
    },
    clamp: (value: number, min: number, max: number) => Math.max(min, Math.min(max, value)),
    round: (value: number, places: number) => Number(value.toFixed(places)),
    normalizeFill(value: any, fallback: string) {
      const type = ['solid', 'linear', 'radial', 'none'].includes(value?.type) ? value.type : 'solid';
      let stops = (value?.stops || [{ id: 'a', color: value?.color || fallback, position: 0 }]).map((stop: any) => ({ ...stop }));
      if (!['solid', 'none'].includes(type) && stops.length < 2) stops.push({ id: 'b', color: stops[0].color, position: 100 });
      if (['solid', 'none'].includes(type)) stops = [stops[0]];
      return { type, angle: value?.angle || 0, stops };
    },
    firstSel: () => layers[PM.sel.layers[0]],
    L: (id: string) => layers[id],
    byName: (name: string) => Object.values(layers).find((layer) => layer.name === name),
    findProp: (layer: any, path: string) => layer.p[path],
    evP: (_layer: any, property: any) => property.v,
    selLayers: () => PM.sel.layers.map((id: string) => layers[id]),
    Edit: { apply: vi.fn() },
    hist: { undo: vi.fn(() => true) },
    cmd: vi.fn(),
    toast: vi.fn(),
    Capabilities: {
      resolveTargets: vi.fn(() => [layers.L1]),
      selectedKeyframes: vi.fn(() => []),
      preview: vi.fn(() => ({ ok: true, message: 'Ready', changes: [] })),
      apply: vi.fn(() => ({ ok: true, message: 'Applied', changes: [] }))
    },
    Script: { canRun: vi.fn(() => true) }
  };
  return { PM, layers };
}

const command = (PM: Record<string, any>, ct: GeneratedControl, value: unknown) => {
  const binding = sourceBinding(PM, ct);
  if (!binding) throw new Error('Expected binding');
  return binding.command(value);
};

describe('generated source bindings', () => {
  it.each([
    ['composition.name', 'name', 'New name'],
    ['composition.width', 'width', 1280],
    ['composition.height', 'height', 720],
    ['composition.fps', 'fps', 60],
    ['composition.duration', 'duration', 12],
    ['composition.shutter', 'shutter', .75],
    ['composition.background', 'background', '#AABBCC'],
    ['composition.backgroundFill', 'backgroundFill', { type: 'none' }]
  ])('maps %s to set_composition.%s', (path, key, value) => {
    const { PM } = fakePM();
    expect(command(PM, control({ target: '$composition', path }), value)).toEqual({
      type: 'set_composition', patch: { [key]: value }
    });
  });

  it('clamps composition work-area endpoints to a one-frame gap', () => {
    const { PM } = fakePM();
    expect(command(PM, control({ target: 'composition', path: 'composition.workArea.start' }), 9)).toEqual({
      type: 'set_composition', patch: { workArea: [7.95, 8] }
    });
    expect(command(PM, control({ target: 'composition', path: 'composition.workArea.end' }), 1)).toEqual({
      type: 'set_composition', patch: { workArea: [2, 2.05] }
    });
  });

  it('coerces stop edits on a solid background to a two-stop linear fill', () => {
    const { PM } = fakePM();
    const result = command(PM, control({ target: '$composition', path: 'composition.background.endColor' }), '#FFFFFF') as any;
    expect(result.type).toBe('set_composition');
    expect(result.patch.backgroundFill).toEqual({
      type: 'linear', angle: 0,
      stops: [{ id: 'a', color: '#101010', position: 0 }, { id: 'b', color: '#FFFFFF', position: 100 }]
    });
  });

  it('maps content and both property prefixes to exact legacy commands', () => {
    const { PM } = fakePM();
    expect(command(PM, control({ target: '$selection', path: 'content.text' }), 'Next')).toEqual({
      type: 'set_content', target: '$selection', patch: { text: 'Next' }
    });
    for (const path of ['properties.opacity', 'transform.opacity']) {
      expect(command(PM, control({ target: '$selection', path }), 55)).toEqual({
        type: 'set_property', target: '$selection', path: 'opacity', value: 55,
        time: 2.5, mode: 'auto', preserveHandEdits: false, markIntent: 'human'
      });
    }
  });

  it.each([
    ['visible', 'on', true],
    ['locked', 'lock', false],
    ['duration', 'dur', 4],
    ['motionBlur', 'mblur', true],
    ['from', 'from', 1]
  ])('reads layer.%s from %s but keeps the public patch key', (key, sourceKey, expected) => {
    const { PM } = fakePM();
    const binding = sourceBinding(PM, control({ target: 'L1', path: `layer.${key}` }))!;
    expect(binding.get()).toBe(expected);
    expect(binding.command('next')).toEqual({ type: 'set_layer', target: 'L1', patch: { [key]: 'next' } });
    expect(sourceKey).toBeTruthy();
  });

  it('resolves selection lazily for getters', () => {
    const { PM } = fakePM();
    const binding = sourceBinding(PM, control({ target: '$selection', path: 'content.text' }))!;
    expect(binding.get()).toBe('One');
    PM.sel.layers = ['L2'];
    expect(binding.get()).toBe('Two');
  });
});

describe('generated scene parameters and buttons', () => {
  it('creates a parameter and follows a project replacement by name', () => {
    const { PM } = fakePM();
    const ct = control({ label: 'Amount', param: 'amount', def: 12, min: 0, max: 20 });
    const parameter = ensureParam(PM, ct);
    expect(parameter).toMatchObject({ name: 'amount', label: 'Amount', control: 'num', value: 12, min: 0, max: 20 });
    PM.proj = { ...PM.proj, params: { amount: { ...parameter, value: 18 } } };
    expect(currentParam(PM, parameter).value).toBe(18);
    expect(sceneParameterCommand(parameter, 7)).toEqual({ type: 'set_scene_parameter', name: 'amount', value: 7 });
  });

  it('strips only the three trust-bearing fields and applies command arrays as generated-ui', async () => {
    const { PM } = fakePM();
    const button = {
      type: 'button', label: 'Run', primary: false, action: null, cmd: '',
      commands: [{ type: 'set_layer', target: 'L1', patch: { visible: false }, overrideLock: true, preserveHandEdits: false, markIntent: 'human', extra: 1 }]
    } as unknown as GeneratedButtonControl;
    expect(stripManifestCommands((button as any).commands)).toEqual([
      { type: 'set_layer', target: 'L1', patch: { visible: false }, extra: 1 }
    ]);
    await resolveButtonAction(PM, button, {});
    expect(PM.Edit.apply).toHaveBeenCalledWith(
      [{ type: 'set_layer', target: 'L1', patch: { visible: false }, extra: 1 }],
      { label: 'Run', origin: 'generated-ui' }
    );
  });

  it('uses the legacy target, script, and keyframe disable predicates', () => {
    const { PM } = fakePM();
    const transform = { type: 'button', label: 'T', primary: false, cmd: '', action: { type: 'transform', mode: 'preview', transform: { selector: { scope: 'selection', types: [] } } } } as any;
    expect(generatedButtonDisabled(PM, transform)).toBe(false);
    PM.Capabilities.resolveTargets.mockReturnValue([]);
    expect(generatedButtonDisabled(PM, transform)).toBe(true);
    expect(generatedButtonDisabled(PM, { ...transform, action: { type: 'script' } })).toBe(false);
    expect(generatedButtonDisabled(PM, { ...transform, action: { type: 'easing' } })).toBe(true);
  });
});

describe('generated preview formatting', () => {
  it('formats frame paths, rounds values, caps the list, and reports overflow', () => {
    const { PM } = fakePM();
    const changes = Array.from({ length: 10 }, (_, index) => ({
      name: `Layer ${index}`, path: index === 0 ? 'layer.duration' : 'properties.opacity', before: index ? 1.23456 : .5, after: index + .98765
    }));
    const preview = formatPreview(PM, { ok: true, message: 'Ten edits', changes }, false)!;
    expect(preview.title).toBe('Preview');
    expect(preview.items[0]?.difference).toBe('10fr → 20fr');
    expect(preview.items[1]?.difference).toBe('1.235 → 1.988');
    expect(preview.items).toHaveLength(9);
    expect(preview.items[8]).toEqual({ description: '2 more changes', more: true });
  });
});
