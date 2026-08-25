import { describe, expect, it } from 'vitest';

import type { PMRegistry } from '../registry';
import { install } from './layout';

function layoutRegistry(): PMRegistry {
  const PM: PMRegistry = {
    h: () => ({ style: {}, classList: { add() {}, remove() {} }, appendChild() {}, append() {} }),
    $: () => null,
    $$: () => [],
    icon: () => '',
    clamp: (v: any, a: any, b: any) => Math.max(a, Math.min(b, v)),
    bus: { on() {}, emit() {} },
    PANELS: {},
    panelInst: {},
    toast() {},
  };
  install(PM);
  return PM;
}

describe('legacy layout install', () => {
  it('accounts for removing the dragged panel before insertion', () => {
    const R = layoutRegistry().Layout.resolveDropIndex;

    expect(R('center', 0, 'center', 2)).toBe(1);
    expect(R('center', 2, 'center', 1)).toBe(1);
    expect(R('center', 1, 'center', 1)).toBeNull();
    expect(R('left', 0, 'right', 3)).toBe(3);
    expect(R('center', 0, 'right', Number.NaN)).toBeNull();
  });

  it('keeps dock hit targets generous and creates missing edge targets', () => {
    const PM = layoutRegistry();
    const rect = (left: number, top: number, right: number, bottom: number) => ({
      left, top, right, bottom, width: right - left, height: bottom - top,
    });
    const docks = [{
      id: 'center',
      rect: rect(96, 44, 1304, 860),
      panels: [{ rect: rect(96, 44, 1304, 440) }, { rect: rect(96, 452, 1304, 860) }],
    }];

    expect(PM.Layout.hitTestDockPlacement(docks, 700, 446)).toEqual({ dockId: 'center', index: 1 });
    expect(PM.Layout.hitTestDockPlacement(docks, 700, 10)).toBeNull();

    const targets = PM.Layout.buildDockDropTargets(docks, rect(0, 44, 1400, 860));
    expect(PM.Layout.hitTestDockPlacement(targets, 18, 400)).toEqual({ dockId: 'left', index: 0 });
  });
});
