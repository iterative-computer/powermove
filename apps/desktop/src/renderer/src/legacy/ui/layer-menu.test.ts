// @vitest-environment happy-dom
import { expect, it, vi } from 'vitest';

import { createKernel } from '../../kernel/registries';
import type { MenuContribution } from '../../kernel/api';
import { installLayerMenu } from './layer-menu';

function harness() {
  const kernel = createKernel();
  const layers = [{ id: 'A', name: 'Layer A', type: 'solid', on: true }, { id: 'B', name: 'Layer B', type: 'solid', on: true }];
  const PM: any = {
    Kernel: kernel,
    proj: { layers },
    sel: { layers: [] as string[] },
    time: 0,
    TYPE_META: {},
    selectLayers: vi.fn((id: string) => { PM.sel.layers = [id]; }),
    selLayers: () => layers.filter(layer => PM.sel.layers.includes(layer.id)),
    expandGroups: (ids: string[]) => ids,
    // Submenus (Parent, Timing) are built when the menu opens.
    L: (id: string) => layers.find(layer => layer.id === id) ?? null,
    wouldCycle: () => false,
    menu: vi.fn()
  };
  installLayerMenu(PM);
  const labels = (items: MenuContribution[]) => items.map(item => item === '-' ? item : 'header' in item ? item.header : item.label);
  return { PM, kernel, layers, labels };
}

it('opens at once with synchronous contributions for the right-clicked layer', () => {
  const { PM, kernel, layers, labels } = harness();
  kernel.contributeMenu('trusted', 'layer:context', (ctx) => [{ label: `Tag ${String(ctx.layerId)}` }]);
  PM.showLayerMenu(layers[1], { clientX: 4, clientY: 5 });
  const items = PM.menu.mock.calls.at(-1)[1] as MenuContribution[];
  expect(Array.isArray(items)).toBe(true);
  expect(labels(items).slice(-2)).toEqual(['-', 'Tag B']);
  expect(PM.menu).toHaveBeenLastCalledWith(document.body, items, { x: 4, y: 5 });
});

it('asks asynchronous contributors afresh per layer, so layer B never gets the items built for layer A', async () => {
  const { PM, kernel, layers, labels } = harness();
  const ran: string[] = [];
  const asked: unknown[] = [];
  kernel.contributeMenu('sandboxed', 'layer:context', async (ctx) => {
    asked.push(ctx.layerId);
    return [{ label: `Tag ${String(ctx.layerId)}`, run: () => { ran.push(String(ctx.layerId)); } }];
  });
  kernel.contributeMenu('sandboxed', 'timeline:context', async (ctx) => [{ label: `Here ${String(ctx.layerId)}` }]);

  PM.showLayerMenu(layers[0], { clientX: 0, clientY: 0 });
  const first = await PM.menu.mock.calls.at(-1)[1] as MenuContribution[];
  PM.showLayerMenu(layers[1], { clientX: 0, clientY: 0 });
  const second = await PM.menu.mock.calls.at(-1)[1] as MenuContribution[];

  // The first open is not empty, and each open shows only its own layer.
  expect(labels(first).slice(-3)).toEqual(['-', 'Tag A', 'Here A']);
  expect(labels(second).slice(-3)).toEqual(['-', 'Tag B', 'Here B']);
  expect(asked).toEqual(['A', 'B']);
  const tag = second.find(item => item !== '-' && 'label' in item && item.label === 'Tag B');
  (tag as { run(): void }).run();
  expect(ran).toEqual(['B']);
});
