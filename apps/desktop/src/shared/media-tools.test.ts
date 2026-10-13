import { describe, expect, it } from 'vitest';

import { formatClock, mediaOutcomeLabels, mediaToolLabels, mediaToolName, mediaToolOutcome, mediaToolSubject, settleMediaLabel } from './media-tools';

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

  it('names the clips the caption tools work on instead of showing a raw id', () => {
    expect(mediaToolSubject('mcp__powermove__generate_captions', { layerIds: ['L1', 'L2'], style: 'boxed' })).toEqual({ layerId: 'L1', clips: 2 });
    expect(mediaToolSubject('generate_captions', { jobId: 'captions-job-1' })).toEqual({});
    expect(mediaToolSubject('export_captions', { layerId: 'C1', format: 'vtt' })).toEqual({ layerId: 'C1', format: 'vtt' });
    expect(mediaToolLabels('generate_captions', { layerId: 'L1', clips: 1 }, 'interview.mov')?.running).toBe('Captioning interview.mov…');
    expect(mediaToolLabels('generate_captions', { layerId: 'L1', clips: 3 }, 'interview.mov')?.done).toBe('Captioned 3 clips');
    expect(mediaToolLabels('generate_captions', {}, null)?.running).toBe('Captioning the clips…');
    expect(mediaToolLabels('export_captions', { layerId: 'C1', format: 'vtt' }, 'Interview Captions')?.done).toBe('Exported Interview Captions as WebVTT');
    expect(settleMediaLabel('export_captions', 'Exporting Interview Captions as SRT…', 'done')).toBe('Exported Interview Captions as SRT');
    expect(settleMediaLabel('generate_captions', 'Captioning interview.mov…', 'error')).toBe('Caption interview.mov');
    expect(mediaToolName('generate_captions')).toBeNull();
  });

  it('formats clock times for labels', () => {
    expect(formatClock(4.2)).toBe('0:04.20');
    expect(formatClock(3723.4)).toBe('1:02:03.40');
    expect(formatClock(-1.5)).toBe('-0:01.50');
  });
});

describe('non-final transcription answers', () => {
  it('reads a pending job or a missing model from the result excerpt', () => {
    expect(mediaToolOutcome('mcp__powermove__transcribe_media', '{"status":"transcribing","asset":{"id":"a1"},"progress":0.42,"note":"…"}')).toEqual({ state: 'pending', progress: 0.42 });
    expect(mediaToolOutcome('transcribe_media', '{"status":"transcribing","asset":{"id":"a1"},"progress":0}')).toEqual({ state: 'pending' });
    expect(mediaToolOutcome('transcribe_media', 'Error: {"status":"model-required","code":"transcription-model-missing"}')).toEqual({ state: 'needs-model' });
  });

  it('leaves real transcripts and other tools alone', () => {
    expect(mediaToolOutcome('transcribe_media', '{"asset":{"id":"a1"},"timeBase":"source","text":"[0:00.00–0:01.00] {\\"status\\":\\"transcribing\\"}"}')).toBeNull();
    expect(mediaToolOutcome('transcribe_media', '{"asset":{"id":"a1"},"text":"{\"status\":\"transcribing\"}"}')).toBeNull();
    expect(mediaToolOutcome('probe_media', '{"status":"transcribing"}')).toBeNull();
    expect(mediaToolOutcome('transcribe_media', undefined)).toBeNull();
  });

  it('words the row for what actually happened', () => {
    expect(mediaOutcomeLabels('Transcribing interview.mov…', '0:30.00–1:10.00', { state: 'pending', progress: 0.42 }))
      .toEqual({ label: 'Still transcribing interview.mov', detail: '0:30.00–1:10.00 · 42%' });
    expect(mediaOutcomeLabels('Transcribing interview.mov…', undefined, { state: 'pending' })).toEqual({ label: 'Still transcribing interview.mov' });
    expect(mediaOutcomeLabels('Transcribing interview.mov…', undefined, { state: 'needs-model' })).toEqual({ label: 'Needs a transcription model for interview.mov' });
    expect(settleMediaLabel('transcribe_media', 'Still transcribing interview.mov', 'done')).toBe('Still transcribing interview.mov');
  });
});
