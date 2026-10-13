// @ts-nocheck -- drives the legacy PM registry and the live agent harness.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { makePM } from '../legacy/__tests__/make-pm';

const client = vi.hoisted(() => ({
  transcriptionStatus: vi.fn(async () => ({ activeModelId: 'base' })),
  ensureTranscriptionModel: vi.fn(async () => true),
  transcribe: vi.fn()
}));
vi.mock('../transcription/client', () => client);
vi.mock('../media/media-path', () => ({ resolveMediaPath: vi.fn(async (id: string) => `/media/${id}.mov`) }));

/* Agent caption generation outlives one tool call, so it runs as a job the
   agent waits on; ending the run cancels it and nothing lands afterwards. */

let PM: any;
let pending: Array<{ options: any; finish: () => void }>;

function editor() {
  vi.stubGlobal('window', { requestAnimationFrame: (resolve: FrameRequestCallback) => resolve(0) });
  PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing', 'gl/shaders', 'assistant/harness');
  PM.proj = PM.mkProject({ name: 'Agent captions', w: 1920, h: 1080, fps: 30, dur: 6 });
  PM.Kernel.services.register('shaderHooks', { syncShaderUniforms() {} });
  const video = PM.mkLayer('video', { name: 'Interview', from: 0, dur: 5 });
  video.id = 'clip';
  video.d.asset = 'asset-1';
  PM.proj.layers.push(video);
  PM.ProjectIndex?.invalidate?.();
  pending = [];
  // Stands in for PM.Captions.generate: transcribes until `finish` is called.
  PM.Captions = {
    generate: vi.fn((_ids: string[], options: any) => new Promise(resolve => {
      pending.push({
        options,
        finish: () => {
          if (options.signal?.aborted) { resolve({ ok: false, message: 'Caption generation was cancelled.' }); return; }
          const applied = options.apply({ type: 'add_captions', name: 'Interview Captions', from: 0, cues: [{ start: 0.5, end: 1.5, text: 'Hello there' }] });
          resolve(applied.ok ? { ok: true, layerId: applied.data.results[0].data.id } : { ok: false, message: applied.message });
        }
      });
    }))
  };
}

const call = (runId: string, tool: string, args: any = {}) => PM.AgentHarness.test.handleLiveAgentTool({ runId, callId: tool, tool, arguments: args, baseRevision: 0 });
const json = (response: any) => JSON.parse(response.content[0].text);
const captionLayers = () => PM.proj.layers.filter((layer: any) => layer.type === 'captions');

beforeEach(() => { vi.useFakeTimers(); editor(); });
afterEach(() => { vi.useRealTimers(); vi.unstubAllGlobals(); });

describe('generate_captions agent tool', () => {
  it('returns a job for long transcriptions and applies the result in the run', async () => {
    const started = call('run-1', 'generate_captions', { layerIds: ['clip'], style: 'boxed' });
    await vi.advanceTimersByTimeAsync(60_000);
    const running = json(await started);
    expect(running).toMatchObject({ status: 'running', jobId: expect.any(String) });
    expect(captionLayers()).toHaveLength(0);

    const waiting = call('run-1', 'generate_captions', { jobId: running.jobId });
    pending[0]!.finish();
    const done = json(await waiting);
    expect(done).toMatchObject({ status: 'done', cues: 1 });
    expect(captionLayers()).toHaveLength(1);
    expect(captionLayers()[0].d.style.preset).toBe('boxed');
    expect((await call('run-1', '__finish_run', { commit: true })).changed).toBe(true);
    expect(captionLayers()).toHaveLength(1);
  });

  it('answers in one call when the transcript is ready in time', async () => {
    const started = call('run-2', 'generate_captions', { layerIds: ['clip'] });
    await vi.advanceTimersByTimeAsync(10);
    pending[0]!.finish();
    expect(json(await started)).toMatchObject({ status: 'done', cues: 1 });
  });

  it('cancels the job when the run ends, so no captions land afterwards', async () => {
    const started = call('run-3', 'generate_captions', { layerIds: ['clip'] });
    await vi.advanceTimersByTimeAsync(60_000);
    const { jobId } = json(await started);
    await call('run-3', '__finish_run', { commit: false });
    expect(pending[0]!.options.signal.aborted).toBe(true);
    pending[0]!.finish();
    await vi.advanceTimersByTimeAsync(0);
    expect(captionLayers()).toHaveLength(0);
    await expect(call('run-3', 'generate_captions', { jobId })).rejects.toThrow(/Unknown caption job/);
  });

  it('refuses to apply into a run whose transaction is gone, even without the abort', async () => {
    const started = call('run-4', 'generate_captions', { layerIds: ['clip'] });
    await vi.advanceTimersByTimeAsync(60_000);
    await started;
    const { options } = pending[0]!;
    await call('run-4', '__finish_run', { commit: true });
    const late = options.apply({ type: 'add_captions', cues: [{ start: 0, end: 1, text: 'Late' }] });
    expect(late.ok).toBe(false);
    expect(captionLayers()).toHaveLength(0);
  });

  it('refuses a second generation while one is running in the same run', async () => {
    const started = call('run-7', 'generate_captions', { layerIds: ['clip'] });
    await vi.advanceTimersByTimeAsync(60_000);
    const { jobId } = json(await started);
    await expect(call('run-7', 'generate_captions', { layerIds: ['clip'] })).rejects.toThrow(new RegExp(`already being generated \\(jobId ${jobId}\\)`));
    expect(PM.Captions.generate).toHaveBeenCalledTimes(1);
    const waiting = call('run-7', 'generate_captions', { jobId });
    pending[0]!.finish();
    expect(json(await waiting)).toMatchObject({ status: 'done' });
  });

  it('keeps jobs private to their run and reports a missing model cleanly', async () => {
    const started = call('run-5', 'generate_captions', { layerIds: ['clip'] });
    await vi.advanceTimersByTimeAsync(60_000);
    const { jobId } = json(await started);
    await expect(call('other-run', 'generate_captions', { jobId })).rejects.toThrow(/Unknown caption job/);
    client.transcriptionStatus.mockResolvedValueOnce({ activeModelId: null });
    await expect(call('run-6', 'generate_captions', { layerIds: ['clip'] })).rejects.toThrow(/^transcription-model-missing/);
    expect(client.ensureTranscriptionModel).toHaveBeenCalled();
  });
});
