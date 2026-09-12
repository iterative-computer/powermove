import { describe, expect, it } from 'vitest';

import {
  clampExportScale,
  exportActionLabel,
  exportFieldSupport,
  normalizeExportDefaults,
  planExport
} from './export-defaults';

describe('normalizeExportDefaults', () => {
  it('fills every field for an empty project', () => {
    expect(normalizeExportDefaults(undefined, 25)).toEqual({
      format: 'mp4', scale: 1, fps: 25, range: 'work', quality: 'high',
      mblur: true, audio: true, alpha: false
    });
  });

  it('rejects unknown formats, qualities and ranges', () => {
    const result = normalizeExportDefaults({
      format: 'mov', quality: 'ultra', range: 'clip', scale: 'huge'
    });
    expect(result).toMatchObject({ format: 'mp4', quality: 'high', range: 'work', scale: 1 });
  });

  it('keeps arbitrary positive scales and clamps runaway ones', () => {
    expect(normalizeExportDefaults({ scale: 0.5208 }).scale).toBeCloseTo(0.5208);
    expect(normalizeExportDefaults({ scale: 7 }).scale).toBe(7);
    expect(normalizeExportDefaults({ scale: 900 }).scale).toBe(16);
    expect(normalizeExportDefaults({ scale: -2 }).scale).toBe(1);
    expect(clampExportScale(0, 2)).toBe(2);
    expect(clampExportScale(0.001)).toBe(0.01);
  });

  it('keeps valid stored choices and clamps the frame rate', () => {
    expect(normalizeExportDefaults({
      format: 'png', scale: 2, fps: 900, range: 'all', quality: 'max',
      mblur: false, audio: false, alpha: true
    })).toEqual({
      format: 'png', scale: 2, fps: 240, range: 'all', quality: 'max',
      mblur: false, audio: false, alpha: true
    });
  });

  it('falls back to 30 fps when neither the value nor the project supplies one', () => {
    expect(normalizeExportDefaults({ fps: 'soon' }, Number.NaN).fps).toBe(30);
  });
});

describe('exportFieldSupport', () => {
  it('only offers the settings each format reads', () => {
    expect(exportFieldSupport('mp4')).toMatchObject({ quality: true, audio: true, alpha: false });
    expect(exportFieldSupport('webm')).toMatchObject({ quality: true, audio: true, alpha: false });
    expect(exportFieldSupport('png')).toMatchObject({ quality: false, audio: false, alpha: true, fps: true });
    expect(exportFieldSupport('still')).toMatchObject({ fps: false, range: false, scale: true, alpha: true });
    expect(Object.values(exportFieldSupport('json')).every((flag) => !flag)).toBe(true);
  });

  it('labels the primary action per format', () => {
    expect(exportActionLabel('webm')).toBe('Export video');
    expect(exportActionLabel('png')).toBe('Export frames');
    expect(exportActionLabel('still')).toBe('Export frame');
    expect(exportActionLabel('json')).toBe('Save project file');
  });
});

describe('planExport', () => {
  const defaults = normalizeExportDefaults({});
  const comp = { w: 1920, h: 1080, dur: 10 };

  it('reports even-rounded output size, frame count and an estimate', () => {
    const plan = planExport({ ...defaults, range: 'all' }, comp);
    expect(plan).toMatchObject({ width: 1920, height: 1080, seconds: 10, frames: 300 });
    expect(plan.note).toContain('MB');
  });

  it('describes MP4 delivery with the H.264 codec', () => {
    const plan = planExport({ ...defaults, format: 'mp4', range: 'all' }, comp);
    expect(plan.note).toContain('H.264');
    expect(plan.note).not.toContain('VP9');
  });

  it('honours the work area only when one is set and valid', () => {
    expect(planExport(defaults, { ...comp, work: [2, 5] }).seconds).toBe(3);
    expect(planExport(defaults, { ...comp, work: [5, 5] }).seconds).toBe(10);
    expect(planExport({ ...defaults, range: 'all' }, { ...comp, work: [2, 5] }).seconds).toBe(10);
  });

  it('rounds arbitrary scales to even pixels like the renderer', () => {
    const plan = planExport({ ...defaults, scale: 1000 / 1920 }, comp);
    expect(plan.width).toBe(1000);
    expect(plan.height % 2).toBe(0);
  });

  it('counts files for image formats', () => {
    expect(planExport({ ...defaults, format: 'still' }, comp).frames).toBe(1);
    expect(planExport({ ...defaults, format: 'png', range: 'all', alpha: true }, comp).note)
      .toBe('300 PNG files · transparent');
  });
});
