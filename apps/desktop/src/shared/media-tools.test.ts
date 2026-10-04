import { describe, expect, it } from 'vitest';

import { formatClock, mediaToolLabels, mediaToolName, mediaToolSubject, settleMediaLabel } from './media-tools';

describe('media tool activity wording', () => {
  it('recognises media tools under every provider naming', () => {
    expect(mediaToolName('mcp__powermove__transcribe_media')).toBe('transcribe_media');
    expect(mediaToolName('Probe_Media')).toBe('probe_media');
    expect(mediaToolName('check_project')).toBe('check_project');
    expect(mediaToolName('render_frames')).toBeNull();
  });

  it('keeps only the arguments a row needs, sanitized', () => {
    expect(mediaToolSubject('sample_media_frames', { assetId: 'a1', times: [1, 2, 3], quality: 'large', junk: 'x' })).toEqual({ assetId: 'a1', frames: 3 });
    expect(mediaToolSubject('transcribe_media', { layerId: 'L1', start: 30, end: 70 })).toEqual({ layerId: 'L1', start: 30, end: 70 });
    expect(mediaToolSubject('media_contact_sheet', { target: 'composition', count: 12 })).toEqual({ target: 'composition', frames: 12 });
    expect(mediaToolSubject('probe_media', { assetId: '<script>' })).toEqual({});
    expect(mediaToolSubject('bash', { command: 'ls' })).toBeUndefined();
    expect(mediaToolSubject('check_project', {})).toBeUndefined();
  });

  it('says what each call does to which clip, then settles the tense', () => {
    expect(mediaToolLabels('transcribe_media', { assetId: 'a1' }, 'interview.mov')).toEqual({ running: 'Transcribing interview.mov…', done: 'Transcribed interview.mov', failed: 'Transcribe interview.mov' });
    expect(mediaToolLabels('mcp__powermove__sample_media_frames', { frames: 12 }, 'b-roll.mp4')?.running).toBe('Sampling 12 frames from b-roll.mp4…');
    expect(mediaToolLabels('sample_media_frames', { auto: true }, 'b-roll.mp4')?.running).toBe('Finding scene changes in b-roll.mp4…');
    expect(mediaToolLabels('media_contact_sheet', { target: 'composition' }, null)?.running).toBe('Building a contact sheet of the composition…');
    expect(mediaToolLabels('media_waveform', { layerId: 'L1', start: 30, end: 70.5 }, null)).toMatchObject({ running: 'Mapping silences in a clip…', detail: '0:30.00–1:10.50' });
    expect(mediaToolLabels('probe_media', undefined, null)?.running).toBe('Probing media…');
    expect(settleMediaLabel('transcribe_media', 'Transcribing interview.mov…', 'done')).toBe('Transcribed interview.mov');
    expect(settleMediaLabel('sample_media_frames', 'Finding scene changes in b.mp4…', 'error')).toBe('Find scene changes in b.mp4');
    expect(settleMediaLabel('transcribe_media', 'Transcribing interview.mov…', 'running')).toBe('Transcribing interview.mov…');
    expect(settleMediaLabel('bash', 'Building things…', 'done')).toBe('Building things…');
  });

  it('formats clock times for labels', () => {
    expect(formatClock(4.2)).toBe('0:04.20');
    expect(formatClock(3723.4)).toBe('1:02:03.40');
    expect(formatClock(-1.5)).toBe('-0:01.50');
  });
});
