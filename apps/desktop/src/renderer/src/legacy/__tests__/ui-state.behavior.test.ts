import { describe, expect, it } from 'vitest';

import { makePM } from './make-pm';

function fixture() {
  const PM = makePM('core/ui-state', 'core/model', 'core/history');
  PM.proj = PM.mkProject({ name: 'UI state fixture', w: 1920, h: 1080, fps: 30, dur: 5 });
  const layer = PM.mkLayer('text', { name: 'Title' });
  layer.id = 'layer-a';
  layer.collapsed = false;
  PM.addLayer(layer, 0);
  const key = { i: 'key-a', t: 0, v: 100, eo: [.33, 0], ei: [.67, 1] };
  layer.p.opacity.kf.push(key);
  PM.UIState.prune(PM.proj);
  return { PM, key, layer };
}

describe('UI-state oracle survivors', () => {
  it('keeps graph handles out of project JSON and history', () => {
    const { PM, key } = fixture();
    PM.hist.clear();
    const before = JSON.stringify(PM.proj);
    const serializedBefore = PM.serialize();
    const canUndoBefore = PM.hist.canUndo();

    PM.hist.begin('Click keyframe');
    PM.UIState.setKeyHandles(key, { ho: [120, 45], hi: [80, 45], pt: [100, 60] });
    PM.hist.commit('Click keyframe');

    expect(JSON.stringify(PM.proj)).toBe(before);
    expect(PM.serialize()).toBe(serializedBefore);
    expect(PM.hist.canUndo()).toBe(canUndoBefore);
    expect(PM.UIState.getKeyHandles(key)).toEqual({
      ho: [120, 45], hi: [80, 45], pt: [100, 60],
    });
  });

  it('prunes every side table when the active project changes', () => {
    const { PM, key, layer } = fixture();
    const effect = { id: 'fx-prune', type: 'blur', on: true, p: {} };
    layer.fx.push(effect);
    PM.UIState.setKeyHandles(key, { pt: [10, 20] });
    PM.UIState.setReveal(layer, ['opacity']);
    PM.UIState.setShaderMeta(layer, { shaderKey: 'shader:old' });
    PM.UIState.setFxOpen(effect, true);
    PM.UIState.setLayerCollapsed(layer, true);
    PM.UIState.setGroupCollapsed(layer, false);

    PM.proj = PM.mkProject({ name: 'Next project', w: 1280, h: 720, fps: 24, dur: 3 });
    PM.bus.emit('project');

    expect(PM.UIState.keyHandles.size).toBe(0);
    expect(PM.UIState.reveal.size).toBe(0);
    expect(PM.UIState.shaderMeta.size).toBe(0);
    expect(PM.UIState.fxOpen.size).toBe(0);
    expect(PM.UIState.layerCollapsed.size).toBe(0);
    expect(PM.UIState.groupCollapsed.size).toBe(0);
  });
});
