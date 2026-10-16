import { describe, expect, it } from 'vitest';

import {
  allPresets,
  BUILT_IN_PRESETS,
  DEFAULT_DELIVERY,
  estimateBytes,
  expandFileName,
  extensionOf,
  fieldSupport,
  formatBytes,
  isDropFrameTimecode,
  legacyFormatId,
  migrateFormat,
  migratePreset,
  normalizeDelivery,
  normalizeTimecode,
  outputDepth,
  safeFileName,
  supportsAlpha,
  wireAlpha,
  wireAudioCoding,
  wireColor,
  wireFormat
} from './delivery';

describe('formats', () => {
  it('migrate today\'s stored ids, realtime capture to frame-exact VP9', () => {
    expect(migrateFormat('prores')).toBe('prores-4444');
    expect(migrateFormat('mp4')).toBe('h264');
    expect(migrateFormat('webm')).toBe('webm-vp9');
    expect(migrateFormat('rec')).toBe('webm-vp9');
    expect(migrateFormat('png')).toBe('png');
    expect(migrateFormat('hevc')).toBe('hevc');
    expect(migrateFormat('mov')).toBe('h264');
    expect(migrateFormat(7, 'still')).toBe('still');
  });

  it('name the current exporter\'s format, or none when only the engine writes it', () => {
    expect(legacyFormatId('prores-4444')).toBe('prores');
    expect(legacyFormatId('h264')).toBe('mp4');
    expect(legacyFormatId('webm-vp9')).toBe('webm');
    expect(legacyFormatId('prores-hq')).toBeNull();
    expect(legacyFormatId('hevc')).toBeNull();
    expect(legacyFormatId('webm-vp8')).toBeNull();
  });

  it('map onto the job engine\'s formats, extensions, alpha, tags, and audio', () => {
    const d = DEFAULT_DELIVERY;
    expect(wireFormat('prores-hq', d)).toEqual({ kind: 'proRes', profile: 'hq' });
    expect(wireFormat('prores-4444xq', d)).toEqual({ kind: 'proRes', profile: 'p4444Xq' });
    expect(wireFormat('hevc', { ...d, highDepth: true, container: 'mov' })).toEqual({ kind: 'hevc', tenBit: true, container: 'mov' });
    expect(wireFormat('webm-vp8', d)).toEqual({ kind: 'webm', codec: 'vp8' });
    expect(wireFormat('png', { ...d, highDepth: true })).toEqual({ kind: 'pngSequence', sixteenBit: true });
    expect(extensionOf('prores-lt', d)).toBe('mov');
    expect(extensionOf('hevc', { ...d, container: 'mov' })).toBe('mov');
    expect(extensionOf('hevc', d)).toBe('mp4');
    expect(extensionOf('png', d)).toBeNull();
    expect(extensionOf('still', d)).toBe('png');
    expect(supportsAlpha('hevc', { highDepth: true })).toBe(false);
    expect(supportsAlpha('hevc', { highDepth: false })).toBe(true);
    expect(supportsAlpha('prores-hq', d)).toBe(false);
    expect(wireAlpha('prores-4444', true, { ...d, alphaMode: 'premultiplied' })).toBe('premultiplied');
    expect(wireAlpha('webm-vp9', true, { ...d, alphaMode: 'premultiplied' })).toBe('straight');
    expect(wireAlpha('h264', true, d)).toBe('none');
    expect(wireColor('png', d)).toEqual({ kind: 'srgbImage' });
    expect(wireColor('still', { ...d, imageColor: 'rec709' })).toEqual({ kind: 'rec709Image' });
    expect(wireColor('h264', { ...d, videoColor: 'broadcast' })).toEqual({ kind: 'rec709Broadcast' });
    expect(wireAudioCoding('prores-hq', { ...d, audioCoding: 'float' })).toBe('pcmFloat');
    expect(wireAudioCoding('h264', { ...d, audioCoding: 'pcm24' })).toBeNull();
    expect(outputDepth('prores-hq', d)).toBe(10);
    expect(outputDepth('png', { highDepth: true })).toBe(16);
  });

  it('offer only the settings a format reads, today\'s only on the current exporter', () => {
    const engine = fieldSupport('hevc', { container: 'mov', highDepth: false }, true);
    expect(engine).toMatchObject({ quality: true, alpha: true, highDepth: true, container: true, audioCoding: true, timecode: true });
    expect(fieldSupport('hevc', { container: 'mp4', highDepth: true }, true)).toMatchObject({ alpha: false, audioCoding: false, timecode: false });
    expect(fieldSupport('prores-hq', DEFAULT_DELIVERY, true)).toMatchObject({ alpha: false, timecode: true, audioCoding: true, quality: false });
    expect(fieldSupport('prores-4444', DEFAULT_DELIVERY, false)).toMatchObject({ alpha: true, alphaMode: false, timecode: false, audioCoding: false });
    expect(fieldSupport('webm-vp9', DEFAULT_DELIVERY, false).alpha).toBe(false);
    expect(fieldSupport('webm-vp9', DEFAULT_DELIVERY, true).alpha).toBe(true);
    expect(fieldSupport('still', DEFAULT_DELIVERY, true)).toMatchObject({ fps: false, range: false, imageColor: true, highDepth: true });
    expect(Object.values(fieldSupport('json', DEFAULT_DELIVERY, true)).every((f) => !f)).toBe(true);
  });
});

describe('presets', () => {
  it('are the five of section 17, first', () => {
    expect(BUILT_IN_PRESETS.map((p) => p.name)).toEqual([
      'Master · ProRes 422 HQ', 'Master with alpha · ProRes 4444', 'Web · H.264', 'Web · HEVC', 'Image sequence · PNG'
    ]);
    expect(BUILT_IN_PRESETS.map((p) => p.options.format)).toEqual(['prores-hq', 'prores-4444', 'h264', 'hevc', 'png']);
  });

  it('migrate today\'s saved presets and drop broken or clashing ones', () => {
    const stored = [
      { id: 'preset1', name: 'Alpha master', options: { format: 'prores', scale: 0.5, alpha: true, fps: 24000 / 1001, range: 'all', quality: 'max', captionsSidecar: 'srt' } },
      { id: 'preset2', name: 'Realtime', options: { format: 'rec', quality: 'draft', mblur: false } },
      { id: 'preset3', name: 'Web · H.264', options: { format: 'mp4' } },
      { id: 'builtin:master', name: 'Spoof', options: { format: 'mp4' } },
      { name: 'No id', options: { format: 'mp4' } },
      'junk'
    ];
    const all = allPresets(stored);
    expect(all.slice(5).map((p) => p.name)).toEqual(['Alpha master', 'Realtime']);
    expect(all[5]!.options).toEqual({ format: 'prores-4444', scale: 0.5, fps: 24000 / 1001, range: 'all', quality: 'max', alpha: true, captionsSidecar: 'srt' });
    expect(all[6]!.options).toEqual({ format: 'webm-vp9', quality: 'draft', mblur: false });
    expect(migratePreset({ id: 'x', name: '', options: {} })).toBeNull();
  });
});

describe('delivery options', () => {
  it('normalize stored values', () => {
    expect(normalizeDelivery(undefined)).toEqual(DEFAULT_DELIVERY);
    expect(normalizeDelivery({ container: 'avi', audioCoding: 'mp3', timecode: '1:00', reel: 'A001\n', overwrite: 'replace', fileName: '  ' }))
      .toMatchObject({ container: 'mp4', audioCoding: 'auto', timecode: '', reel: 'A001', overwrite: 'replace', fileName: '{project}' });
    expect(normalizeTimecode('01:00:00:00')).toBe('01:00:00:00');
    expect(normalizeTimecode('00:01:00;02')).toBe('00:01:00;02');
    expect(normalizeTimecode('00:61:00:00')).toBe('');
    expect(isDropFrameTimecode('00:01:00;02')).toBe(true);
    expect(isDropFrameTimecode('00:01:00:02')).toBe(false);
  });
});

describe('file name tokens', () => {
  const ctx = {
    project: 'Launch', composition: 'Main cut', preset: 'Web · H.264', format: 'h264' as const,
    width: 1920, height: 1080, fps: 30000 / 1001, now: new Date(2026, 9, 16, 9, 5, 7)
  };

  it('expand every token and keep unknown ones', () => {
    expect(expandFileName('{project} {composition} {size} {fps} {format} {date} {time}', ctx))
      .toBe('Launch Main cut 1920x1080 29.97 H.264 2026-10-16 09-05-07');
    expect(expandFileName('{project}_{width}x{height}_{unknown}', ctx)).toBe('Launch_1920x1080_{unknown}');
    expect(expandFileName('{preset}', ctx)).toBe('Web · H.264');
  });

  it('make bare, safe names', () => {
    expect(expandFileName('../{project}/x:y', ctx)).toBe('_Launch_x_y');
    expect(expandFileName('', ctx)).toBe('Launch');
    expect(safeFileName('a\u0000b')).toBe('a_b');
    expect(new TextEncoder().encode(safeFileName('é'.repeat(200))).length).toBeLessThanOrEqual(180);
  });
});

describe('size estimates', () => {
  const base = { delivery: DEFAULT_DELIVERY, width: 1920, height: 1080, fps: 30000 / 1001, seconds: 10, frames: 300, quality: 'high' as const, alpha: false, audio: false };

  it('follow the bit rate, Apple\'s ProRes rates, and PNG sizes', () => {
    expect(estimateBytes({ ...base, format: 'h264' })).toBe(20_000_000);
    expect(estimateBytes({ ...base, format: 'h264', audio: true })).toBe(20_000_000 + 320_000);
    expect(estimateBytes({ ...base, format: 'prores-hq' })).toBe(275_000_000);
    expect(estimateBytes({ ...base, format: 'prores-hq', width: 3840, height: 2160 })).toBe(1_100_000_000);
    expect(estimateBytes({ ...base, format: 'prores-hq', audio: true })).toBe(275_000_000 + 2_880_000);
    expect(estimateBytes({ ...base, format: 'png', alpha: true })).toBe(1920 * 1080 * 4 * 300 / 2);
    expect(estimateBytes({ ...base, format: 'still' })).toBe(1920 * 1080 * 3 / 2);
    expect(formatBytes(1_100_000_000)).toBe('≈ 1.1 GB');
    expect(formatBytes(20_320_000)).toBe('≈ 20 MB');
    expect(formatBytes(3_110)).toBe('≈ 3 KB');
  });
});
