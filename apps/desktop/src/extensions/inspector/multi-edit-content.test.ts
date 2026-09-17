import { describe, expect, it, vi } from 'vitest';
import type { EditCommand, PowermoveAPI } from 'powermove';
import { createInspectorEdit, inspectorMixed, translateContentPatch } from './multi-edit';

type FixtureLayer = {
  id: string;
  type: string;
  name: string;
  from: number;
  dur: number;
  lock: boolean;
  p: Record<string, unknown>;
  d: Record<string, any>;
  fx: unknown[];
  masks: unknown[];
};

const mkLayer = (id: string, type: string, d: Record<string, any>): FixtureLayer =>
  ({ id, type, name: id, from: 0, dur: 10, lock: false, p: {}, d, fx: [], masks: [] });

function makeFixture(layers: FixtureLayer[]) {
  const byId = new Map(layers.map((layer) => [layer.id, layer]));
  const dispatch = vi.fn((input: EditCommand | EditCommand[]) => {
    for (const command of Array.isArray(input) ? input : [input]) {
      if (command.type !== 'set_content') continue;
      const layer = byId.get(String(command.target));
      if (layer) Object.assign(layer.d, command.patch);
    }
    return { ok: true };
  });
  const api = {
    model: { layer: (id: string) => byId.get(id) ?? null },
    selection: { layers: () => layers.map((layer) => layer.id), keys: () => [] },
    groups: { ancestors: () => [] },
    transport: { time: () => 0 },
    anim: { findProp: () => null, evP: () => null },
    edit: { begin: vi.fn(), dispatch, apply: dispatch, commit: vi.fn(), cancel: vi.fn(), mutate: vi.fn() }
  } as unknown as PowermoveAPI;
  return { api, layers, byId, dispatch, edit: createInspectorEdit(api) };
}

const fillCommand = (target: string, color: string): EditCommand =>
  ({ type: 'set_content', target, patch: { color } } as EditCommand);

describe('bulk content editing across layer types', () => {
  it('applies a fill change to every selected layer that has a colour, regardless of type', () => {
    const fixture = makeFixture([
      mkLayer('text', 'text', { color: '#111111', text: 'Hello' }),
      mkLayer('shape', 'shape', { color: '#222222', shape: 'rect' }),
      mkLayer('solid', 'solid', { color: '#333333' })
    ]);
    fixture.edit.apply(fillCommand('text', '#ff0000'));
    expect(fixture.layers.map((layer) => layer.d.color)).toEqual(['#ff0000', '#ff0000', '#ff0000']);
  });

  it('skips selected layers whose type has no such content field', () => {
    const fixture = makeFixture([
      mkLayer('text', 'text', { color: '#111111' }),
      mkLayer('image', 'image', { fit: 'cover' })
    ]);
    fixture.edit.apply(fillCommand('text', '#00ff00'));
    expect(fixture.byId.get('text')!.d.color).toBe('#00ff00');
    expect(fixture.byId.get('image')!.d).toEqual({ fit: 'cover' });
  });

  it('shares dimension edits across mixed types', () => {
    const fixture = makeFixture([
      mkLayer('shape', 'shape', { w: 100, color: '#111111' }),
      mkLayer('image', 'image', { w: 50, fit: 'cover' })
    ]);
    fixture.edit.apply({ type: 'set_content', target: 'shape', patch: { w: 200 } } as EditCommand);
    expect(fixture.byId.get('image')!.d.w).toBe(200);
  });

  it('reports mixed fills across a cross-type selection', () => {
    const fixture = makeFixture([
      mkLayer('text', 'text', { color: '#111111' }),
      mkLayer('shape', 'shape', { color: '#222222' })
    ]);
    const binding = {
      mode: 'command' as const,
      label: 'Fill',
      command: (value: unknown) => fillCommand('text', String(value))
    };
    expect(inspectorMixed(fixture.api as any, binding, '#111111')).toBe(true);
  });

  it('keeps non-animatable content fields within a single type', () => {
    const text = mkLayer('text', 'text', {});
    const shape = mkLayer('shape', 'shape', {});
    expect(translateContentPatch(text as any, shape as any, { asset: 'a1' })).toBeNull();
    expect(translateContentPatch(text as any, { ...text, id: 'text2' } as any, { asset: 'a1' })).toEqual({ asset: 'a1' });
  });
});
