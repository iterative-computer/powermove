import { describe, expect, it } from 'vitest';
import { makePM } from './make-pm';
import { sanitizeProject } from '../../core/validate/project';
import { contentFields, resolveContent } from '../core/content-properties';

function editor() {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
  PM.proj = PM.mkProject({ dur: 10 });
  PM.time = 0;
  return PM;
}

describe('editable content animation', () => {
  for (const [type, fields] of Object.entries(contentFields)) {
    it(`animates and persists each ${type} content field`, () => {
      const PM = editor();
      const layer = PM.mkLayer(type); PM.proj.layers.push(layer);
      for (const key of fields) {
        const initial = layer.d[key]?.v ?? layer.d[key];
        if (initial == null) continue;
        const next = typeof initial === 'number' ? initial + 1 : typeof initial === 'boolean' ? !initial : initial;
        expect(PM.Edit.apply({ type: 'set_property', target: layer.id, path: `c.${key}`, value: initial, time: 0, mode: 'keyframe' }).ok).toBe(true);
        expect(PM.Edit.apply({ type: 'set_property', target: layer.id, path: `c.${key}`, value: next, time: 2 }).ok).toBe(true);
        expect(resolveContent(PM, layer, 2)[key]).toEqual(next);
        expect(PM.allProps(layer).filter((item: any) => item.key === `c.${key}`)).toHaveLength(1);
        const saved = sanitizeProject(JSON.parse(PM.serialize()).proj).layers[0] as any;
        expect(saved.d[key].kf).toHaveLength(2);
      }
    });
  }

  it('interpolates numeric content, preserves animation during ordinary edits, and undoes it', () => {
    const PM = editor();
    const layer = PM.mkLayer('shape'); PM.proj.layers.push(layer);
    for (const [time, value] of [[0, 100], [2, 300]]) expect(PM.Edit.apply({ type: 'set_property', target: layer.id, path: 'c.w', value, time, mode: 'keyframe' }).ok).toBe(true);
    expect(resolveContent(PM, layer, 1).w).toBeCloseTo(200);
    PM.time = 1;
    expect(PM.Edit.apply({ type: 'set_content', target: layer.id, patch: { w: 250, color: '#FF0000' } }).ok).toBe(true);
    expect(layer.d.w.kf).toHaveLength(3);
    expect(resolveContent(PM, layer, 1).w).toBe(250);
    PM.hist.undo();
    const restored = PM.L(layer.id);
    expect(resolveContent(PM, restored, 1).w).toBeCloseTo(200);
    const cloned = PM.cloneLayer(restored);
    expect(cloned.d.w.kf.map((key: any) => key.i)).not.toEqual(restored.d.w.kf.map((key: any) => key.i));
  });

  it('keeps structural asset references outside the property model', () => {
    const PM = editor(); const layer = PM.mkLayer('image'); PM.proj.layers.push(layer);
    expect(PM.Edit.apply({ type: 'set_property', target: layer.id, path: 'c.asset', value: 'asset', mode: 'keyframe' }).ok).toBe(false);
    expect(layer.d.asset).toBeNull();
  });

  it('persists discrete blend and mask settings as real channels', () => {
    const PM = editor(); const layer = PM.mkLayer('shape'); PM.proj.layers.push(layer);
    layer.masks.push(PM.mkMask('rect', PM.proj));
    for (const [path, value] of [['l.blend', 'screen'], ['l.mblur', true], [`m.${layer.masks[0].id}.shape`, 'ellipse'], [`m.${layer.masks[0].id}.mode`, 'subtract']]) {
      expect(PM.Edit.apply({ type: 'set_property', target: layer.id, path, value, time: 2, mode: 'keyframe' }).ok).toBe(true);
      const saved = sanitizeProject(JSON.parse(PM.serialize()).proj).layers[0];
      expect(PM.findProp(saved, path).kf).toHaveLength(1);
    }
  });
});

it('keeps visibility structural at every time and undoable', () => {
  const PM = editor(); const layer = PM.mkLayer('shape'); PM.proj.layers.push(layer);
  expect(PM.Edit.apply({ type: 'set_property', target: layer.id, path: 'l.on', value: false, time: 2, mode: 'keyframe' }).ok).toBe(false);
  expect(PM.findProp(layer, 'l.on')).toBeNull();
  expect(PM.allProps(layer).some((entry: any) => entry.key === 'l.on')).toBe(false);
  PM.time = 1;
  expect(PM.Edit.apply({ type: 'set_layer', target: layer.id, patch: { visible: false } }).ok).toBe(true);
  expect(PM.L(layer.id).on).toBe(false);
  expect(PM.active(PM.L(layer.id), 0)).toBe(false);
  expect(PM.active(PM.L(layer.id), 2)).toBe(false);
  PM.hist.undo();
  expect(PM.active(PM.L(layer.id), 1)).toBe(true);
});

it('smoothly interpolates visual colors while text and hold keys stay discrete', () => {
  const PM = editor(); const layer = PM.mkLayer('text'); PM.proj.layers.push(layer);
  for (const path of ['c.color', 'c.text']) for (const [time, value] of [[0, '#ff0000'], [2, '#0000ff']]) {
    expect(PM.Edit.apply({ type: 'set_property', target: layer.id, path, value, time, mode: 'keyframe' }).ok).toBe(true);
  }
  expect(resolveContent(PM, layer, 1).color).toBe('#800080');
  expect(resolveContent(PM, layer, 1).text).toBe('#ff0000');
  layer.d.color.kf[0].hold = true; PM.touch();
  expect(resolveContent(PM, layer, 1).color).toBe('#ff0000');
});
