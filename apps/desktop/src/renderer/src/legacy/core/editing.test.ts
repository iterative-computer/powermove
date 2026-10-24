import { describe, expect, it } from 'vitest';

import type { PMRegistry } from '../registry';
import { install } from './editing';
import { makePM } from '../__tests__/make-pm';

describe('legacy editing install', () => {
  it('can create a layer before zero through the same editing commands', () => {
    const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
    PM.proj = PM.mkProject({ fps: 30, dur: 10 });
    expect(PM.Edit.apply({ type: 'add_layer', id: 'early', layerType: 'solid', from: -3, duration: 5 }).ok).toBe(true);
    expect(PM.L('early')).toMatchObject({ from: -3, dur: 5 });
  });

  it('moves a layer before zero without changing its duration or local keys, with undo and redo', () => {
    const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
    PM.proj = PM.mkProject({ fps: 30, dur: 10 });
    const layer = PM.mkLayer('solid', { from: 1, dur: 3 });
    PM.addLayer(layer, 0);
    PM.setKey(layer, 'opacity', 2, 50);
    PM.hist.clear();

    expect(PM.Edit.apply({ type: 'set_layer', target: layer.id, patch: { from: -7 } }).ok).toBe(true);
    expect(PM.L(layer.id)).toMatchObject({ from: -7, dur: 3 });
    expect(PM.L(layer.id).p.opacity.kf[0].t).toBe(1);
    PM.hist.undo();
    expect(PM.L(layer.id).from).toBe(1);
    PM.hist.redo();
    expect(PM.L(layer.id).from).toBe(-7);
  });

  it('installs the frozen source-edit vocabulary', () => {
    const PM: PMRegistry = {};
    install(PM);

    expect(PM.Edit.operations.set_property).toEqual({
      target: 'layer', fields: ['path', 'value', 'time', 'mode', 'ease'],
    });
    expect(PM.Edit.operations.set_content).toEqual({ target: 'layer', fields: ['patch'] });
    expect(PM.Edit.operations.transform_layers).toEqual({
      target: 'layer-collection', fields: ['transform', 'state'],
    });
    expect(PM.Edit.operations.set_transition).toEqual({
      target: 'layer', fields: ['layer', 'edge', 'transition'],
    });
    expect(Object.keys(PM.Edit.operations)).toHaveLength(26);
  });
});
