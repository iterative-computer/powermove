// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest';

import { parentMenuItems } from './layer-menu';
import { installParentPickwhip } from './parent-pickwhip';

function parentingUI() {
  const child = { id: 'child', name: 'Child', type: 'solid', parent: null, lock: false };
  const group = { id: 'group', name: 'Rig Group', type: 'group', parent: null, lock: false };
  const layers = [child, group];
  const apply = vi.fn();
  const PM: any = {
    proj: { layers },
    TYPE_META: { solid: { transform: true }, group: { transform: true } },
    L: (id: string) => layers.find(layer => layer.id === id) ?? null,
    wouldCycle: () => false,
    Edit: { apply },
  };
  return { PM, child, group, apply };
}

afterEach(() => document.body.replaceChildren());

describe('parenting UI', () => {
  it('offers group layers as parents in the parent menu', () => {
    const { PM, child, group, apply } = parentingUI();
    const item = parentMenuItems(PM, [child.id]).find(candidate => candidate.label === group.name);

    expect(item).toBeTruthy();
    item.run();
    expect(apply).toHaveBeenCalledExactlyOnceWith(
      [{ type: 'set_layer', target: child.id, patch: { parent: group.id } }],
      { label: 'Parent layers', origin: 'timeline' },
    );
  });

  it('accepts a group layer as a pickwhip target', () => {
    const { PM, child, group, apply } = parentingUI();
    PM.TL = { layerAtPoint: vi.fn(() => group) };
    PM.Viewer = { layerAtPoint: vi.fn(() => null) };
    installParentPickwhip(PM);

    PM.beginParentPick(new PointerEvent('pointerdown', { clientX: 20, clientY: 30 }), [child.id]);
    window.dispatchEvent(new PointerEvent('pointerup', { clientX: 120, clientY: 140 }));

    expect(apply).toHaveBeenCalledExactlyOnceWith(
      [{ type: 'set_layer', target: child.id, patch: { parent: group.id } }],
      { label: 'Parent layers', origin: 'interface' },
    );
  });
});
