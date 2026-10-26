import { describe, expect, it } from 'vitest';

import type { PMRegistry } from '../registry';
import { install } from './media';

function mediaRegistry(): PMRegistry {
  const PM: PMRegistry = {};
  install(PM);
  return PM;
}

describe('legacy media install', () => {
  it('derives poster storage next to its media bytes', () => {
    const PM = mediaRegistry();

    expect(PM.MediaImport.posterKeyFor('media:v2:123:abc')).toBe('media:v2:123:abc:poster');
  });

  it('preserves source time for video trim timing', () => {
    const PM = mediaRegistry();
    const video = { type: 'video', from: 2, d: { trim: 1, speed: 2 } };

    expect(PM.MediaTiming.trimAtStart(video, 5)).toBe(7);
    expect(PM.MediaTiming.earliestStart(video)).toBe(1.5);
  });

  it('trims media before zero while respecting the beginning of its source', () => {
    const PM = mediaRegistry();
    const video = { type: 'video', from: -3, d: { trim: 1, speed: 2 } };
    expect(PM.MediaTiming.earliestStart(video)).toBe(-3.5);
    expect(PM.MediaTiming.trimAtStart(video, -3.5)).toBe(0);
    expect(PM.MediaTiming.trimAtStart(video, -2)).toBe(3);
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

it('coalesces typed effect image/LUT asset IDs while deletion preserves recoverable references',()=>{
 const PM=mediaRegistry(),project:any={assets:{old:{id:'old',kind:'lut'},current:{id:'current',kind:'lut'}},layers:[{id:'owner',fx:[{inputs:{lut:{kind:'asset',assetId:'old'}}}]}],comps:{child:{layers:[{id:'child-owner',fx:[{inputs:{texture:{kind:'asset',assetId:'old'}}}]}]}}};
 expect(PM.MediaImport.referenceCount(project,'old')).toBe(2);expect(PM.MediaImport.coalesce(project,'current',['old'])).toBe(2);expect(project.layers[0].fx[0].inputs.lut.assetId).toBe('current');expect(project.comps.child.layers[0].fx[0].inputs.texture.assetId).toBe('current');expect(PM.MediaImport.removeAsset(project,'current').removedLayerIds).toEqual([]);expect(project.layers[0].fx[0].inputs.lut.assetId).toBe('current');
});
