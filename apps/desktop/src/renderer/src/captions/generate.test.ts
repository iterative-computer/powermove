import { describe, expect, it, vi } from 'vitest';
import { TRANSCRIPTION_MODEL_MISSING, type Transcript } from '../../../shared/transcription';
import { captionsFromSpeech, transcriptWords } from './generate';
import { captionFileInfo } from './install';
import { linearClip } from './time-map';

const transcript = (words: Array<[string, number, number]>, language = 'en'): Transcript => ({
  modelId: 'test', language, duration: 60,
  segments: [{ text: words.map(([text]) => text).join(' '), start: words[0]![1], end: words.at(-1)![2], words: words.map(([text, start, end]) => ({ text, start, end })) }]
});

const clip = (overrides: Record<string, unknown> = {}) => ({ id: 'v1', name: 'Interview', type: 'video', from: 10, dur: 6, d: { asset: 'asset-1', trim: 20, speed: 1 }, ...overrides });

function deps(result: Transcript | Error, ready = true) {
  return {
    ensureModel: vi.fn(async () => ready),
    resolvePath: vi.fn(async (id: string) => `/media/${id}.mov`),
    transcribe: vi.fn(async () => { if (result instanceof Error) throw result; return result; }),
    clipTiming: (layer: any) => linearClip(layer.from, layer.dur, layer.d.trim, layer.d.speed),
    progress: vi.fn()
  };
}

describe('captionsFromSpeech', () => {
  it('transcribes only the used source span and maps words into the composition', async () => {
    const d = deps(transcript([['Before.', 18, 19], ['Hello', 21, 21.4], ['there.', 21.5, 22], ['After', 27, 28]]));
    const result = await captionsFromSpeech([clip()], d);
    expect(d.ensureModel).toHaveBeenCalledWith('Captions for “Interview”');
    expect(d.transcribe).toHaveBeenCalledWith(expect.objectContaining({ path: '/media/asset-1.mov', start: 20, end: 26, wordTimestamps: true }), expect.any(Function), undefined);
    expect(result.status).toBe('ok');
    if (result.status !== 'ok') return;
    expect(result.language).toBe('en');
    expect(result.cues.map(cue => cue.text)).toEqual(['Hello there.']);
    expect(result.cues[0]!.start).toBeCloseTo(11);
    expect(result.cues[0]!.words!.map(word => word.start)).toEqual([11, 11.5]);
  });

  it('reports a missing model from the sheet or from the engine', async () => {
    expect(await captionsFromSpeech([clip()], deps(transcript([['x', 21, 22]]), false))).toEqual({ status: 'model-missing' });
    const missing = Object.assign(new Error('no model'), { code: TRANSCRIPTION_MODEL_MISSING });
    expect(await captionsFromSpeech([clip()], deps(missing))).toEqual({ status: 'model-missing' });
  });

  it('rejects layers without media and reports silence', async () => {
    await expect(captionsFromSpeech([{ type: 'text', d: {} }], deps(transcript([['x', 0, 1]])))).rejects.toThrow(/audio or video/);
    expect(await captionsFromSpeech([clip()], deps(transcript([['far', 50, 51]])))).toEqual({ status: 'empty' });
  });

  it('honours Cancel even when the engine resolves afterwards', async () => {
    const controller = new AbortController();
    const d = deps(transcript([['Hello', 21, 21.4]]));
    d.transcribe.mockImplementation(async () => { controller.abort(); return transcript([['Hello', 21, 21.4]]); });
    await expect(captionsFromSpeech([clip()], { ...d, signal: controller.signal })).rejects.toMatchObject({ name: 'AbortError' });
    // An empty transcript after Cancel is a cancellation, not silence.
    const empty = new AbortController();
    d.transcribe.mockImplementation(async () => { empty.abort(); return { modelId: 'test', duration: 0, segments: [] }; });
    await expect(captionsFromSpeech([clip()], { ...d, signal: empty.signal })).rejects.toMatchObject({ name: 'AbortError' });
  });

  it('treats segments without word timings as one word', () => {
    expect(transcriptWords({ modelId: 'm', duration: 2, segments: [{ text: ' Whole line ', start: 1, end: 2, words: [] }] }))
      .toEqual([{ text: 'Whole line', start: 1, end: 2 }]);
  });
});

describe('caption file names', () => {
  it('reads a language tag from the file name', () => {
    expect(captionFileInfo('Interview.en.srt')).toEqual({ name: 'Interview', language: 'en' });
    expect(captionFileInfo('Episode 1.pt-BR.vtt')).toEqual({ name: 'Episode 1', language: 'pt-BR' });
    expect(captionFileInfo('notes.srt')).toEqual({ name: 'notes' });
    expect(captionFileInfo('clip.mov.srt')).toEqual({ name: 'clip' });
    expect(captionFileInfo('Song.mp3.vtt')).toEqual({ name: 'Song' });
  });
});
