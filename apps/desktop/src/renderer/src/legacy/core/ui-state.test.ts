import { describe, expect, it } from 'vitest';

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
