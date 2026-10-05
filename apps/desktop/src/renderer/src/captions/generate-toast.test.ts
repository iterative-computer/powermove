// @ts-nocheck -- drives the legacy PM registry like the command suites.
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { makePM } from '../legacy/__tests__/make-pm';
import { installCaptions } from './install';

const client = vi.hoisted(() => ({
  transcriptionStatus: vi.fn(),
  ensureTranscriptionModel: vi.fn(async () => true),
  transcribe: vi.fn()
}));
vi.mock('../transcription/client', () => client);
vi.mock('../media/media-path', () => ({ resolveMediaPath: vi.fn(async (id: string) => `/media/${id}.mov`) }));

/* The sticky "Generating captions" toast only appears once a model is ready
   and is always replaced by a completed toast, whatever ends the run. */

let PM: any;
let toasts: Array<{ text: string; options: any }>;
const sticky = () => toasts.filter(item => item.options.sticky);
const last = () => toasts.at(-1)!;

beforeEach(() => {
  PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing', 'gl/shaders');
  PM.proj = PM.mkProject({ name: 'Toast', w: 1920, h: 1080, fps: 30, dur: 6 });
  PM.Kernel.services.register('shaderHooks', { syncShaderUniforms() {} });
  const video = PM.mkLayer('video', { name: 'Interview', from: 0, dur: 5 });
  video.d.asset = 'asset-1';
  PM.proj.layers.push(video);
  toasts = [];
  PM.toast = (text: string, _ms: number, options: any) => { toasts.push({ text, options }); };
  installCaptions(PM);
  client.ensureTranscriptionModel.mockReset().mockResolvedValue(true);
  client.transcribe.mockReset();
});

const clip = () => PM.proj.layers.find((layer: any) => layer.type === 'video');

describe('caption generation progress', () => {
  it('shows no progress while the model download sheet is open and nothing runs', async () => {
    client.ensureTranscriptionModel.mockResolvedValue(false);
    const result = await PM.Captions.generate([clip().id]);
    expect(result.status).toBe('model-missing');
    expect(sticky()).toEqual([]);
    expect(last().options.completed).toBe(true);
  });

  it('completes the progress toast when the project changes mid-transcription', async () => {
    client.transcribe.mockImplementation(async () => {
      PM.proj = PM.mkProject({ name: 'Other', w: 1920, h: 1080, fps: 30, dur: 6 });
      return { modelId: 'base', duration: 5, segments: [{ text: 'Hi', start: 0, end: 1, words: [{ text: 'Hi', start: 0, end: 1 }] }] };
    });
    const result = await PM.Captions.generate([clip().id]);
    expect(result.ok).toBe(false);
    expect(sticky().length).toBeGreaterThan(0);
    expect(last()).toMatchObject({ text: expect.stringMatching(/project changed/), options: { completed: true } });
  });

  it('adds the captions and completes the toast on success; an outside abort cancels', async () => {
    client.transcribe.mockResolvedValue({ modelId: 'base', language: 'en', duration: 5, segments: [{ text: 'Hello there.', start: 0.5, end: 1.5, words: [{ text: 'Hello', start: 0.5, end: 1 }, { text: 'there.', start: 1, end: 1.5 }] }] });
    const result = await PM.Captions.generate([clip().id]);
    expect(result.ok).toBe(true);
    expect(PM.L(result.layerId).d.cues.map((cue: any) => cue.text)).toEqual(['Hello there.']);
    expect(last()).toMatchObject({ text: 'Added 1 caption', options: { completed: true } });

    const controller = new AbortController();
    client.transcribe.mockImplementation(async () => { controller.abort(); return { modelId: 'base', duration: 5, segments: [] }; });
    const cancelled = await PM.Captions.generate([clip().id], { signal: controller.signal });
    expect(cancelled).toMatchObject({ ok: false, message: 'Caption generation was cancelled.' });
    expect(last()).toMatchObject({ text: 'Caption generation cancelled', options: { completed: true } });
  });
});
