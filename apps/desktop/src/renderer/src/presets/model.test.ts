import { describe, expect, it, vi } from 'vitest';

import { applyPreset, describeSkipped, initialSelection, presetChoices, presetDetail, selectionSummary, uniquePresetName } from './model';

const P = (v: unknown, kf: unknown[] = []) => ({ v, kf, expr: null });

function fakePM() {
  const blur = { id: 'fx-1', type: 'blur', on: true, p: { amount: P(8, [{ t: 0 }, { t: 1 }]) } };
  const layer: any = {
    id: 'layer-1', name: 'Card', type: 'shape',
    p: { 'position.x': P(0, [{ t: 0 }, { t: 1 }]), 'position.y': P(0, [{ t: 0 }, { t: 1 }]), rotation: P(0), opacity: P(100, [{ t: 0 }]) },
    fx: [blur],
  };
  const PM: Record<string, any> = {
    time: 3,
    PAIRS: [['Position', ['position.x', 'position.y'], 'position'], ['Rotation', ['rotation'], 'rotation'], ['Opacity', ['opacity'], 'opacity']],
    CH: { 'position.x': { label: 'Position X' }, rotation: { label: 'Rotation' }, opacity: { label: 'Opacity' } },
    FX: { blur: { label: 'Gaussian Blur' } },
    effectDef: (fx: any) => ({ blur: { label: 'Gaussian Blur' } } as any)[fx.type],
    allProps: (L: any) => [
      ...Object.entries(L.p).map(([key, prop]) => ({ key, prop, label: key, group: 'Transform' })),
      { key: 'c.size', prop: P(10), label: 'Size', group: 'Content' },
      { key: 'fx-1.amount', prop: blur.p.amount, label: 'amount', group: 'blur' },
      { key: 'm.mask-1.feather', prop: P(0), label: 'feather', group: 'Mask 1' },
    ],
    L: (id: string) => (id === layer.id ? layer : null),
    firstSel: () => layer,
    Presets: { list: () => [{ id: 'p1', name: 'Card Preset' }], get: (id: string) => (id === 'p1' ? { id: 'p1', name: 'Pop', effects: [], properties: [] } : null) },
    Edit: { apply: vi.fn(() => ({ ok: true, data: { results: [{ data: { skipped: [] } }] } })) },
    Inspector: { refresh: vi.fn() },
    invalidate: vi.fn(),
    toast: vi.fn(),
  };
  return { PM, layer };
}

describe('preset choices', () => {
  it('offers effects and portable properties as inspector rows, never masks or effect params', () => {
    const { PM, layer } = fakePM();
    const choices = presetChoices(PM, layer);
    expect(choices.effects).toEqual([{ id: 'fx-1', label: 'Gaussian Blur', keys: 2 }]);
    expect(choices.properties.map((row) => [row.label, row.paths, row.keys])).toEqual([
      ['Position', ['position.x', 'position.y'], 4],
      ['Rotation', ['rotation'], 0],
      ['Opacity', ['opacity'], 1],
      ['Size', ['c.size'], 0],
    ]);
  });

  it('starts from what was pointed at, else the effects and animated properties', () => {
    const { PM, layer } = fakePM();
    const choices = presetChoices(PM, layer);
    expect(initialSelection(choices)).toEqual({ effectIds: ['fx-1'], propertyIds: ['Position', 'Opacity'] });
    expect(initialSelection(choices, { paths: ['rotation'] })).toEqual({ effectIds: [], propertyIds: ['Rotation'] });
    expect(initialSelection(choices, { paths: ['fx-1.amount'] })).toEqual({ effectIds: ['fx-1'], propertyIds: [] });
    expect(initialSelection(choices, { effectIds: ['fx-1'] })).toEqual({ effectIds: ['fx-1'], propertyIds: [] });
  });

  it('previews what Save writes in one sentence', () => {
    const { PM, layer } = fakePM();
    const choices = presetChoices(PM, layer);
    expect(selectionSummary(choices, { effectIds: ['fx-1'], propertyIds: ['Opacity'] }, true))
      .toBe('Saves Gaussian Blur and Opacity with 3 keyframes, placed from the playhead when applied.');
    expect(selectionSummary(choices, { effectIds: ['fx-1'], propertyIds: [] }, false))
      .toBe('Saves Gaussian Blur as their values at the playhead, without keyframes.');
    expect(selectionSummary(choices, { effectIds: [], propertyIds: ['Rotation'] }, true)).toBe('Saves Rotation as they are now.');
    expect(selectionSummary(choices, { effectIds: [], propertyIds: [] }, true)).toBe('Choose at least one effect or property to save.');
  });

  it('describes saved presets, unique names and skipped items', () => {
    const { PM } = fakePM();
    expect(presetDetail(PM, { id: 'a', name: 'A', effects: [{ type: 'blur', parameters: { amount: { kf: [1] } }, enabled: true }], properties: [{ path: 'rotation', channel: { kf: [] } }] }))
      .toBe('Gaussian Blur and Rotation · animated');
    expect(uniquePresetName(PM, 'Card Preset')).toBe('Card Preset 2');
    expect(uniquePresetName(PM, 'Fresh')).toBe('Fresh');
    expect(describeSkipped(['Effect unavailable: tint'])).toBe('Skipped one item (effect unavailable tint).');
    expect(describeSkipped(['Effect unavailable: tint', 'Property unavailable: c.size'])).toBe('Skipped 2 items that don’t fit this layer (effect unavailable tint, …).');
  });
});

describe('applyPreset', () => {
  it('applies as one labelled edit at the playhead and reports skipped items gently', () => {
    const { PM } = fakePM();
    expect(applyPreset(PM, 'p1')).toBe(true);
    expect(PM.Edit.apply).toHaveBeenCalledWith({ type: 'apply_preset', target: 'layer-1', preset: 'p1', time: 3 }, { label: 'Apply “Pop”', origin: 'fx-browser' });
    expect(PM.toast).toHaveBeenLastCalledWith('Applied “Pop” to Card', 2200, { error: false, key: 'preset-apply' });

    PM.Edit.apply.mockReturnValueOnce({ ok: true, data: { results: [{ data: { skipped: ['Effect unavailable: tint'] } }] } });
    applyPreset(PM, 'p1');
    expect(PM.toast.mock.calls.at(-1)[0]).toBe('Applied “Pop” to Card. Skipped one item (effect unavailable tint).');

    PM.Edit.apply.mockReturnValueOnce({ ok: false, message: 'No compatible effects or properties in preset. Effect unavailable: tint' });
    expect(applyPreset(PM, 'p1')).toBe(false);
    expect(PM.toast.mock.calls.at(-1)[0]).toBe('Nothing in “Pop” fits Card. Effect unavailable: tint');
  });

  it('asks for a layer or reports a missing preset without editing', () => {
    const { PM } = fakePM();
    PM.firstSel = () => null;
    expect(applyPreset(PM, 'p1')).toBe(false);
    expect(PM.toast).toHaveBeenLastCalledWith('Select a layer to apply “Pop”');
    expect(applyPreset(PM, 'missing', 'layer-1')).toBe(false);
    expect(PM.Edit.apply).not.toHaveBeenCalled();
  });
});
