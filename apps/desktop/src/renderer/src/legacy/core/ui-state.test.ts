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

/** A project loaded with one group, as a saved file would open. */
function groupRegistry(collapsed: boolean) {
  const { PM } = uiStateRegistry();
  const layer: any = { id: 'group-a', type: 'group', collapsed, p: {}, fx: [], masks: [] };
  PM.proj = { layers: [layer], comps: {} };
  PM.UIState.prune(PM.proj);
  return { PM, layer };
}

describe('legacy ui-state install', () => {
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

  it('starts an open group with its animated properties showing', () => {
    const { PM, layer } = groupRegistry(false);

    expect(PM.UIState.getGroupCollapsed(layer)).toBe(false);
    expect(PM.UIState.getLayerCollapsed(layer)).toBe(false);
  });

  it('starts a closed group with its properties closed too', () => {
    const { PM, layer } = groupRegistry(true);

    expect(PM.UIState.getGroupCollapsed(layer)).toBe(true);
    expect(PM.UIState.getLayerCollapsed(layer)).toBe(true);
  });

  it('keeps group hierarchy disclosure separate from property disclosure afterwards', () => {
    const { PM, layer } = groupRegistry(false);

    PM.UIState.setLayerCollapsed(layer, true);
    expect(PM.UIState.getGroupCollapsed(layer)).toBe(false);
    PM.UIState.setGroupCollapsed(layer, true);
    PM.UIState.setLayerCollapsed(layer, false);

    expect(PM.UIState.getGroupCollapsed(layer)).toBe(true);
    expect(PM.UIState.getLayerCollapsed(layer)).toBe(false);
  });
});
