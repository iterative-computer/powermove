import { describe, expect, it, vi } from 'vitest';
vi.mock('./UpgradeSheet.svelte', () => ({ default: {} }));
import { projectShareSnapshot } from './plan';
import { formatBytes, usagePercent } from '../../../shared/aerolite';

describe('share snapshot', () => {
  it('captures facts, not project data', () => {
    const snapshot = projectShareSnapshot({ name: '  Launch film ', w: 1920, h: 1080, fps: 30, dur: 15,
      comps: { a: { name: 'Main', dur: 15, layers: [{}, {}] } },
      layers: [{ name: 'Logo', type: 'shape', from: 1, dur: 4, secretKeyframes: [1, 2, 3] }, { type: 'text' }] });
    expect(snapshot).toEqual({ kind: 'project', title: 'Launch film', width: 1920, height: 1080, fps: 30, durationSeconds: 15,
      compositions: [{ name: 'Main', durationSeconds: 15, layerCount: 2 }],
      layers: [{ name: 'Logo', type: 'shape', inSeconds: 1, outSeconds: 5 }, { name: 'Layer', type: 'text' }] });
  });
  it('fills sane defaults and caps sizes', () => {
    const snapshot = projectShareSnapshot({ name: '', layers: Array.from({ length: 500 }, () => ({ name: 'x'.repeat(500) })) });
    expect(snapshot.title).toBe('Untitled project');
    expect(snapshot.layers).toHaveLength(300);
    expect(snapshot.layers[0]!.name).toHaveLength(120);
    expect([snapshot.width, snapshot.height, snapshot.fps]).toEqual([1920, 1080, 30]);
  });
  it('formats usage without dollars', () => {
    expect([usagePercent(0), usagePercent(0.004), usagePercent(0.426), usagePercent(1.3)]).toEqual(['0%', '<1%', '43%', '100%']);
    expect([formatBytes(12 * 1024), formatBytes(3.5 * 1024 ** 2), formatBytes(1024 ** 3)]).toEqual(['12 KB', '3.5 MB', '1 GB']);
  });
});
