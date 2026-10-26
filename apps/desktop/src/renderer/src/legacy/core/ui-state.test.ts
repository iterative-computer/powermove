import { describe, expect, it, vi } from 'vitest';

import type { PMRegistry } from '../registry';
import { install } from './ui-state';

function uiStateRegistry() {
  const listeners = new Map<string, Set<() => void>>();
  const PM: PMRegistry = {
    bus: {
      on(name: string, fn: () => void) {
        if (!listeners.has(name)) listeners.set(name, new Set());
        listeners.get(name)!.add(fn);
      },
      emit(name: string) {
        for (const fn of listeners.get(name) || []) fn();
      },
    },
  };
  install(PM);
  const key: any = { i: 'key-a', t: 0, v: 100 };
  const effect: any = { id: 'fx-a', type: 'blur', on: true, open: true, p: {} };
  const layer: any = {
    id: 'layer-a',
    collapsed: false,
    p: { opacity: { kf: [key] } },
    fx: [effect],
    masks: [],
  };
  PM.proj = { layers: [layer], comps: {} };
  PM.UIState.prune(PM.proj);
  return { PM, key, layer, effect };
}

describe('legacy ui-state install', () => {
  it('prunes another project at once, and within one project only after its edits settle', () => {
    vi.useFakeTimers();
    try {
      const { PM, layer } = uiStateRegistry();
      let evaluation = 0, structure = 0;
      PM.evaluationVersion = () => evaluation;
      PM.ProjectIndex = { generation: () => structure };
      const removed: any = { i: 'key-b', t: 1, v: 0 };
      layer.p.opacity.kf.push(removed);
      PM.UIState.setKeyHandles(removed, { ho: [1, 1] });
      PM.bus.emit('project');
      const version = PM.UIState.timelineVersion();
      layer.p.opacity.kf.pop();
      // A composition setting: neither the evaluator nor the layer index moved.
      PM.bus.emit('project'); PM.bus.emit('layers');
      vi.runAllTimers();
      expect(PM.UIState.timelineVersion()).toBe(version);
      expect(PM.UIState.keyHandles.has('key-b')).toBe(true);
      // An edit: pruned once its burst of announcements has stopped.
      evaluation++; PM.bus.emit('layers');
      vi.advanceTimersByTime(300);
      structure++; PM.bus.emit('project');
      vi.advanceTimersByTime(300);
      expect(PM.UIState.keyHandles.has('key-b')).toBe(true);
      vi.advanceTimersByTime(200);
      expect(PM.UIState.keyHandles.has('key-b')).toBe(false);
      // Another project is pruned at once.
      PM.UIState.setKeyHandles(removed, { ho: [1, 1] });
      PM.proj = { layers: [], comps: {} };
      PM.bus.emit('project');
      expect(PM.UIState.keyHandles.size).toBe(0);
    } finally { vi.useRealTimers(); }
  });
  it('versions timeline row changes without invalidating on read-only access or repeated values', () => {
    const { PM, layer, key } = uiStateRegistry();
    const initial = PM.UIState.timelineVersion();
    PM.UIState.getLayerCollapsed(layer);
    PM.UIState.getReveal(layer);
    PM.UIState.getKeyHandles(key);
    PM.UIState.setLayerCollapsed(layer, false);
    expect(PM.UIState.timelineVersion()).toBe(initial);

    PM.UIState.setLayerCollapsed(layer, true);
    const collapsed = PM.UIState.timelineVersion();
    expect(collapsed).toBeGreaterThan(initial);
    PM.UIState.setLayerCollapsed(layer, true);
    expect(PM.UIState.timelineVersion()).toBe(collapsed);

    layer._reveal = ['opacity'];
    const revealed = PM.UIState.timelineVersion();
    expect(revealed).toBeGreaterThan(collapsed);
    PM.UIState.setReveal(layer, ['opacity']);
    expect(PM.UIState.timelineVersion()).toBe(revealed);
    PM.UIState.setGroupCollapsed(layer, true);
    expect(PM.UIState.timelineVersion()).toBeGreaterThan(revealed);
  });

  it('keeps handle and shader metadata outside project JSON', () => {
    const { PM, key, layer } = uiStateRegistry();
    const udefs = [{ name: 'amount', label: 'Amount', control: 'slider', min: 0, max: 1 }];
    PM.UIState.setKeyHandles(key, { ho: [120, 45], hi: [80, 45], pt: [100, 60] });
    PM.UIState.setShaderMeta(layer, { udefs, shaderKey: 'sh:layer-a:123' });

    expect(PM.UIState.getKeyHandles(key)).toEqual({ ho: [120, 45], hi: [80, 45], pt: [100, 60] });
    expect(PM.UIState.getShaderMeta(layer).shaderKey).toBe('sh:layer-a:123');
    expect(PM.UIState.getShaderMeta(layer).udefs).toEqual(udefs);
    expect(JSON.stringify(PM.proj).includes('_shaderKey')).toBe(false);
    expect(JSON.stringify(PM.proj).includes('_udefs')).toBe(false);
  });

  it('redirects legacy transient writers to side-table accessors', () => {
    const { PM, layer, effect } = uiStateRegistry();
    layer._reveal = ['position.x'];
    effect.open = false;

    expect(PM.UIState.getReveal(layer)).toEqual(['position.x']);
    expect(PM.UIState.getFxOpen(effect)).toBe(false);
    expect(JSON.stringify(PM.proj).includes('"open"')).toBe(false);
  });

  it('keeps group hierarchy disclosure separate from property disclosure', () => {
    const { PM, layer } = uiStateRegistry();
    layer.type = 'group';
    PM.UIState.prune(PM.proj);

    expect(PM.UIState.getLayerCollapsed(layer)).toBe(true);
    expect(PM.UIState.getGroupCollapsed(layer)).toBe(false);
    PM.UIState.setGroupCollapsed(layer, true);

    expect(PM.UIState.getGroupCollapsed(layer)).toBe(true);
    expect(PM.UIState.getLayerCollapsed(layer)).toBe(true);
  });
});
