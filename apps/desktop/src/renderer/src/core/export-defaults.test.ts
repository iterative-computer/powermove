import { describe, expect, it } from 'vitest';

import { DEFAULT_DELIVERY } from '../../../shared/delivery';
import {
  clampExportScale,
  exportActionLabel,
  exportFieldSupport,
  normalizeExportDefaults,
  planExport,
  storedExportDefaults
} from './export-defaults';

describe('normalizeExportDefaults', () => {
  it('fills every field for an empty project', () => {
    expect(normalizeExportDefaults(undefined, 25)).toEqual({
      format: 'h264', scale: 1, fps: 25, range: 'work', quality: 'high',
      mblur: true, audio: true, alpha: false, delivery: DEFAULT_DELIVERY
    });
  });

  it('rejects unknown formats, qualities and ranges', () => {
    const result = normalizeExportDefaults({
      format: 'mov', quality: 'ultra', range: 'clip', scale: 'huge'
    });
    expect(result).toMatchObject({ format: 'h264', quality: 'high', range: 'work', scale: 1 });
    // Today's ids migrate; realtime capture becomes the frame-exact VP9 export.
    expect(normalizeExportDefaults({ format: 'prores' }).format).toBe('prores-4444');
    expect(normalizeExportDefaults({ format: 'rec' }).format).toBe('webm-vp9');
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
      mblur: false, audio: false, alpha: true, delivery: DEFAULT_DELIVERY
    });
  });

  it('falls back to 30 fps when neither the value nor the project supplies one', () => {
    expect(normalizeExportDefaults({ fps: 'soon' }, Number.NaN).fps).toBe(30);
  });
});

describe('exportFieldSupport', () => {
  it('only offers the settings each format reads', () => {
    expect(exportFieldSupport('h264')).toMatchObject({ quality: true, audio: true, alpha: false });
    expect(exportFieldSupport('webm-vp9')).toMatchObject({ quality: true, audio: true, alpha: false });
    expect(exportFieldSupport('webm-vp9', DEFAULT_DELIVERY, true)).toMatchObject({ alpha: true });
    expect(exportFieldSupport('prores-hq', DEFAULT_DELIVERY, true)).toMatchObject({ quality: false, timecode: true, audioCoding: true });
    expect(exportFieldSupport('png')).toMatchObject({ quality: false, audio: false, alpha: true, fps: true });
    expect(exportFieldSupport('still')).toMatchObject({ fps: false, range: false, scale: true, alpha: true });
    expect(Object.values(exportFieldSupport('json')).every((flag) => !flag)).toBe(true);
  });

  it('labels the primary action per format', () => {
    expect(exportActionLabel('webm-vp9')).toBe('Export video');
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
    const plan = planExport({ ...defaults, format: 'h264', range: 'all' }, comp);
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
      .toBe('300 PNG files · transparent · ≈ 1.2 GB');
    // PNG takes odd sizes; video stays even.
    expect(planExport({ ...defaults, format: 'png', scale: 0.5005 }, comp).width).toBe(961);
    expect(planExport({ ...defaults, format: 'h264', scale: 0.5005 }, comp).width).toBe(960);
  });

  it('estimates ProRes and HEVC sizes', () => {
    expect(planExport({ ...defaults, format: 'prores-hq', range: 'all', audio: false }, comp).note).toBe('≈ 275 MB · ProRes 422 HQ');
    expect(planExport({ ...defaults, format: 'hevc', range: 'all', audio: false, delivery: { ...DEFAULT_DELIVERY, highDepth: true } }, comp).note).toBe('≈ 20 MB · 16 Mbps HEVC 10-bit');
  });
});

it('clamps the project frame rate when stored fps is absent or invalid', () => {
  expect(normalizeExportDefaults({}, 900).fps).toBe(240);
  expect(normalizeExportDefaults({ fps: -1 }, 900).fps).toBe(240);
});

describe('the document form of export defaults (issue P1)', () => {
  it('stores today\'s format ids where they exist and reads back the migrated ids', () => {
    for (const [today, sheet] of [['prores', 'prores-4444'], ['mp4', 'h264'], ['webm', 'webm-vp9'], ['png', 'png'], ['still', 'still'], ['web', 'web'], ['json', 'json']] as const) {
      const read = normalizeExportDefaults({ format: today }, 30);
      expect(read.format).toBe(sheet);
      const stored = storedExportDefaults(read);
      expect(stored.format).toBe(today);
      expect(normalizeExportDefaults(stored, 30)).toEqual(read);
    }
  });

  it('keeps a stored alias that still means the same format, and the new id for formats today has none', () => {
    expect(storedExportDefaults(normalizeExportDefaults({ format: 'rec' }, 30), { format: 'rec' }).format).toBe('rec');
    expect(storedExportDefaults(normalizeExportDefaults({ format: 'mp4' }, 30), { format: 'rec' }).format).toBe('mp4');
    expect(storedExportDefaults(normalizeExportDefaults({ format: 'prores-hq' }, 30)).format).toBe('prores-hq');
    expect(storedExportDefaults(normalizeExportDefaults({ format: 'prores-4444' }, 30), { format: 'prores-4444' }).format).toBe('prores');
  });

  it('round-trips today\'s stored values unchanged apart from additive fields', () => {
    const today = { format: 'prores', scale: 1, fps: 24, range: 'all', quality: 'max', mblur: false, audio: true, alpha: true };
    const stored = storedExportDefaults(normalizeExportDefaults(today, 24), today);
    expect({ ...stored, delivery: undefined }).toEqual({ ...today, delivery: undefined });
  });
});
