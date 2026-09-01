import { describe, expect, it } from 'vitest';

import type { PMRegistry } from '../registry';
import { install } from './media';

function mediaRegistry(): PMRegistry {
  const PM: PMRegistry = {};
  install(PM);
  return PM;
}

describe('legacy media install', () => {
  it('preserves source time for video trim timing', () => {
    const PM = mediaRegistry();
    const video = { type: 'video', from: 2, d: { trim: 1, speed: 2 } };

    expect(PM.MediaTiming.trimAtStart(video, 5)).toBe(7);
    expect(PM.MediaTiming.earliestStart(video)).toBe(1.5);
  });

  it('preserves order without exceeding bounded work concurrency', async () => {
    const PM = mediaRegistry();
    let active = 0;
    let peak = 0;
    const output = await PM.MediaImport.mapBounded([1, 2, 3, 4, 5], 2, async (value: number) => {
      active++;
      peak = Math.max(peak, active);
      await Promise.resolve();
      active--;
      return value * 10;
    });

    expect(output).toEqual([10, 20, 30, 40, 50]);
    expect(peak).toBe(2);
  });

  it('tracks, relinks, and removes model assets referenced by structured extension layers', () => {
    const PM = mediaRegistry();
    const project: any = {
      assets: { old: { id: 'old', kind: 'model' }, current: { id: 'current', kind: 'model' } },
      comps: {},
      layers: [{ id: 'mesh', type: 'extension', d: { data: { assetId: 'old' } } }]
    };
    expect(PM.MediaImport.referenceCount(project, 'old')).toBe(1);
    expect(PM.MediaImport.coalesce(project, 'current', ['old'])).toBe(1);
    expect(project.layers[0].d.data.assetId).toBe('current');
    expect(project.assets.old).toBeUndefined();
    expect(PM.MediaImport.removeAsset(project, 'current')).toEqual({ removedLayers: 1, removedLayerIds: ['mesh'] });
  });
});
