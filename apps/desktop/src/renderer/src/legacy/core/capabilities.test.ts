import { describe, expect, it } from 'vitest';

import type { PMRegistry } from '../registry';
import { install } from './capabilities';

function capabilitiesRegistry(): PMRegistry {
  const PM: PMRegistry = {
    TYPE_META: { text: {} },
    clamp: (value: number, min: number, max: number) => Math.max(min, Math.min(max, value)),
    round: (value: number, places = 0) => Number(value.toFixed(places)),
  };
  install(PM);
  return PM;
}

describe('legacy capabilities install', () => {
  it('rejects unsafe or unbounded generated actions', () => {
    const PM = capabilitiesRegistry();

    expect(PM.Capabilities.sanitizeTransform({ selector: { scope: 'selection' }, edits: [
      { path: '__proto__.polluted', value: 1 },
    ] })).toBeNull();
    expect(PM.Capabilities.sanitizeControlAction({ type: 'transform', mode: 'apply', transform: {
      selector: { scope: 'selection' }, edits: [{ path: 'layer.from', value: { op: 'runJavaScript', args: ['alert(1)'] } }],
    } })).toBeNull();
    expect(PM.Capabilities.sanitizeControlAction({ type: 'history', command: 'deleteAll' })).toBeNull();
    expect(PM.Capabilities.panelRecipe('layer-stagger').state.offsetFrames).toBe(2);
  });
});
