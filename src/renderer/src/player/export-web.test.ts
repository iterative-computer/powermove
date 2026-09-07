// @vitest-environment happy-dom
import { describe, expect, it } from 'vitest';
import { createEngine } from './player';
import { buildWebExport, inspectWebExport } from './export-web';
import { zipFiles } from './archive';
import { exportActionLabel, exportFieldSupport, normalizeExportDefaults, planExport } from '../core/export-defaults';

function fixture() {
  const PM = createEngine(null);
  PM.proj = PM.mkProject({ name: '</script><script>bad()</script>', w: 100, h: 100, dur: 2 });
  PM.proj.notes = 'private notes'; PM.proj.edits = [{ secret: true }];
  const layer = PM.mkLayer('solid', { name: 'Box' }, PM.proj);
  layer.p['position.x'].kf = [PM.KF(0, 10), PM.KF(1, 90)];
  PM.proj.layers.push(layer);
  PM.MediaStore = { get: async () => null };
  return PM;
}

describe('portable web export', () => {
  it('exports owned scene source, a standalone runtime, and safe preview markup', async () => {
    const PM = fixture(), before = PM.serialize();
    const result = await buildWebExport(PM);
    const layer = result.scene.project.layers[0]!;
    expect(layer.type !== 'audio' && layer.type !== 'group' && layer.p['position.x'].kf.length).toBe(2);
    expect(result.scene.project).not.toHaveProperty('notes');
    expect(result.scene.project).not.toHaveProperty('edits');
    expect(PM.serialize()).toBe(before);
    expect(new TextDecoder().decode(result.files.get('index.html'))).not.toContain('<script>bad()');
    expect(result.files.get('player.js')!.length).toBeGreaterThan(10000);
    expect(result.files.has('PowermoveAnimation.jsx')).toBe(true);
    const handoff = JSON.parse(new TextDecoder().decode(result.files.get('handoff.json')));
    expect(handoff.composition.durationSeconds).toBe(2);
    expect(handoff.referenceTimesSeconds).toEqual([0, 1, 2 - 1 / PM.proj.fps]);
    for (const file of Object.values<string>(handoff.files)) expect(result.files.has(file)).toBe(true);
    expect(result.files.has(handoff.entrypoint)).toBe(true);
    expect(result.files.has('HANDOFF_PROMPT.txt')).toBe(true);
    expect(new DataView(result.bytes.buffer).getUint32(0, true)).toBe(0x04034b50);
  });

  it('fails on missing definitions and media instead of flattening or omitting them', async () => {
    const PM = fixture();
    PM.proj.layers[0].fx.push({ type: 'missing', p: {}, on: true });
    expect(inspectWebExport(PM).errors[0]).toContain('missing');
    await expect(buildWebExport(PM)).rejects.toThrow('missing');
    PM.proj.layers[0].fx = [];
    PM.proj.layers.push(PM.mkLayer('image', { d: { asset: 'missing' } }, PM.proj));
    await expect(buildWebExport(PM)).rejects.toThrow('Missing media');
  });

  it('preserves missing effect placeholders without blocking the editor-equivalent export', async () => {
    const PM = fixture();
    const effect = { id: 'swap', type: 'word-slide-swap', on: true, missing: true,
      p: { progress: PM.P(0, { kf: [PM.KF(0, 0), PM.KF(.6, 100)] }) } };
    PM.proj.layers[0].fx.push(effect);
    const before = PM.serialize();
    const result = await buildWebExport(PM);
    expect(result.scene.effects).toEqual([]);
    expect((result.scene.project.layers[0] as any).fx[0]).toEqual(effect);
    expect(result.scene.warnings.join('\n')).toContain('word-slide-swap');
    expect(PM.serialize()).toBe(before);
  });

  it('captures only used media and removes workstation-specific metadata', async () => {
    const PM = fixture();
    PM.proj.assets = { image: { id: 'image', kind: 'image', name: 'image.png', path: '/private/a', storageKey: 'local-only' }, unused: { id: 'unused' } };
    PM.proj.layers.push(PM.mkLayer('image', { d: { asset: 'image' } }, PM.proj));
    PM.MediaStore.get = async () => new Blob(['pixels'], { type: 'image/png' });
    const result = await buildWebExport(PM);
    expect(Object.keys(result.scene.assets)).toEqual(['image']);
    expect(result.scene.project.assets.image).not.toHaveProperty('path');
    expect(result.scene.project.assets.image).not.toHaveProperty('storageKey');
  });

  it('rejects unsafe zip entry paths', () => {
    expect(() => zipFiles(new Map([['../escape', new Uint8Array()]]))).toThrow('Invalid export path');
  });

  it('uses full-composition source delivery without video settings', () => {
    const opts = normalizeExportDefaults({ format: 'web' }, 30);
    expect(opts.format).toBe('web');
    expect(Object.values(exportFieldSupport('web')).every(v => !v)).toBe(true);
    expect(exportActionLabel('web')).toBe('Export code');
    expect(planExport(opts, { w: 100, h: 100, dur: 2, work: [0, 1] }).note).toContain('full composition');
  });
});
