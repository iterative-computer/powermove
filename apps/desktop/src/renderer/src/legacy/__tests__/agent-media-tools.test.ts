import { afterEach, describe, expect, it, vi } from 'vitest';

import type { PMRegistry } from '../registry';
import { makePM } from './make-pm';
import { agentMediaSource, layerTiming } from '../assistant/media-source';
import { checkProject } from '../assistant/project-check';

vi.mock('../../media/media-path', () => ({
  resolveMediaSource: vi.fn(async (assetId: string) => ({ path: `/media/${assetId}.mov`, origin: 'source', assetId, name: `${assetId}.mov`, kind: assetId.startsWith('aud') ? 'audio' : 'video', proxy: false }))
}));

function editor(): PMRegistry {
  vi.stubGlobal('window', { requestAnimationFrame: (resolve: FrameRequestCallback) => resolve(0) });
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing', 'assistant/harness');
  PM.proj = PM.mkProject({ name: 'Media tools', w: 1920, h: 1080, fps: 30, dur: 6 });
  return PM;
}

function runtime(entries: Record<string, any>) {
  const map = new Map(Object.entries(entries));
  return { map, get: (id: string) => map.get(id), loading: new Set<string>(), errors: new Map<string, string>(), cloud: { get: () => undefined } };
}

afterEach(() => vi.unstubAllGlobals());

describe('__media_source', () => {
  it('maps constant and retimed clips from composition to source time', () => {
    const PM = editor();
    const clip = PM.mkLayer('video', { from: 2, dur: 3, d: { asset: 'v1', trim: PM.P(1), speed: PM.P(2) } });
    PM.proj.layers = [clip]; PM.ProjectIndex.invalidate();
    expect(layerTiming(PM, clip)).toEqual({ constant: true, samples: [[2, 1], [5, 7]] });
    clip.d.speed = PM.P(1);
    PM.setKeyOn(clip.d.speed, 0, 1, 'linear', 30);
    PM.setKeyOn(clip.d.speed, 3, 3, 'linear', 30);
    const animated = layerTiming(PM, clip);
    expect(animated.constant).toBe(false);
    expect(animated.samples.length).toBeGreaterThan(50);
    expect(animated.samples[0]).toEqual([2, 1]);
    expect(animated.samples.at(-1)![1]).toBeGreaterThan(6);
  });

  it('resolves a layer to its asset file plus timing, and explains bad targets', async () => {
    const PM = editor();
    PM.proj.assets = { v1: { id: 'v1', name: 'v1.mov', kind: 'video', dur: 10 } };
    PM.assets = runtime({ v1: { dur: 10, hasAudio: true } });
    const clip = PM.mkLayer('video', { name: 'Interview', from: 1, dur: 4, d: { asset: 'v1', trim: PM.P(2), speed: PM.P(1) } });
    const title = PM.mkLayer('text', { name: 'Title' });
    PM.proj.layers = [clip, title]; PM.ProjectIndex.invalidate();
    const source = await agentMediaSource(PM, { layerId: clip.id });
    expect(source).toEqual({
      path: '/media/v1.mov', origin: 'source',
      asset: { id: 'v1', name: 'v1.mov', kind: 'video', duration: 10, hasAudio: true, proxy: false },
      layer: { id: clip.id, name: 'Interview', type: 'video', from: 1, duration: 4, timing: { constant: true, samples: [[1, 2], [5, 6]] }, sourceStart: 2, sourceEnd: 6 }
    });
    await expect(agentMediaSource(PM, { layerId: title.id })).rejects.toThrow(/text layer/);
    await expect(agentMediaSource(PM, { layerId: 'missing' })).rejects.toThrow(/get_project_state/);
    await expect(agentMediaSource(PM, { layerId: clip.id, assetId: 'other' })).rejects.toThrow(/not other/);
    await expect(agentMediaSource(PM, {})).rejects.toThrow(/assetId/);
  });

  it('is served by the live tool harness', async () => {
    const PM = editor();
    PM.proj.assets = { aud1: { id: 'aud1', name: 'aud1.mov', kind: 'audio', dur: 3 } };
    const response = await PM.AgentHarness.test.handleLiveAgentTool({ runId: 'media-src', callId: 'c1', tool: '__media_source', arguments: { assetId: 'aud1' }, baseRevision: 0 });
    expect(JSON.parse(response.content[0].text)).toMatchObject({ path: '/media/aud1.mov', asset: { kind: 'audio', hasAudio: true } });
  });
});

describe('__composition_info', () => {
  it('answers composition size and timing without acknowledging changes the agent has not read', async () => {
    const PM = editor();
    PM.proj.revision = 6;
    const call = (tool: string, args: any = {}) => PM.AgentHarness.test.handleLiveAgentTool({ runId: 'comp-info', callId: tool, tool, arguments: args, baseRevision: 3 });
    const info = JSON.parse((await call('__composition_info')).content[0].text);
    expect(info).toEqual({ width: 1920, height: 1080, fps: 30, duration: 6, workArea: [0, 6] });
    // A contact sheet of the composition must not let an edit through against state the agent never saw.
    await expect(call('apply_commands', { commands: [{ type: 'add_layer', id: 'fresh', layerType: 'text' }] })).rejects.toThrow('revision 3');
    expect(PM.L('fresh')).toBeFalsy();
  });
});

describe('check_project', () => {
  it('reports a clean composition as ok', () => {
    const PM = editor();
    PM.proj.layers = [PM.mkLayer('solid', { from: 0, dur: 6 })]; PM.ProjectIndex.invalidate();
    expect(checkProject(PM)).toMatchObject({ ok: true, issues: [], span: [0, 6], sampledEveryFrames: 1 });
  });

  it('finds empty frames, invisible and stray layers, media problems and clipping', () => {
    const PM = editor();
    PM.proj.assets = {
      short: { id: 'short', name: 'short.mov', kind: 'video', dur: 2 },
      loud: { id: 'loud', name: 'loud.wav', kind: 'audio', dur: 6 },
      gone: { id: 'gone', name: 'gone.mov', kind: 'video', dur: 6 }
    };
    PM.assets = runtime({ short: { dur: 2 }, loud: { dur: 6, peaks: new Float32Array(600).fill(0.5), audioDur: 6 } });
    PM.assets.errors.set('gone', 'Unsupported codec');
    const a = PM.mkLayer('shape', { name: 'A', from: 0, dur: 2 });
    const b = PM.mkLayer('shape', { name: 'B', from: 3, dur: 3 });
    const ghost = PM.mkLayer('shape', { name: 'Ghost', from: 0, dur: 6 }); ghost.p.opacity.v = 0;
    const hidden = PM.mkLayer('shape', { name: 'Hidden', from: 0, dur: 6 }); hidden.on = false;
    const late = PM.mkLayer('shape', { name: 'Late', from: 9, dur: 2 });
    const long = PM.mkLayer('video', { name: 'Long', from: 3, dur: 3, d: { asset: 'short', trim: PM.P(0), speed: PM.P(1) } });
    const broken = PM.mkLayer('video', { name: 'Broken', from: 0, dur: 1, d: { asset: 'gone', trim: PM.P(0), speed: PM.P(1) } });
    const orphan = PM.mkLayer('video', { name: 'Orphan', from: 0, dur: 1, d: { asset: 'nowhere', trim: PM.P(0), speed: PM.P(1) } });
    const music = PM.mkLayer('audio', { name: 'Music', from: 0, dur: 6, d: { asset: 'loud', trim: PM.P(0), gain: PM.P(3) } });
    PM.proj.layers = [a, b, ghost, hidden, late, long, broken, orphan, music]; PM.ProjectIndex.invalidate();
    const report = checkProject(PM);
    expect(report.ok).toBe(false);
    const by = (code: string) => report.issues.filter((issue) => issue.code === code);
    expect(by('empty-frames')).toEqual([expect.objectContaining({ severity: 'error', ranges: [[2, 3]] })]);
    expect(by('never-visible').map((issue) => issue.layerId)).toEqual([ghost.id]);
    expect(by('outside-composition').map((issue) => issue.layerId)).toEqual([late.id]);
    expect(by('past-media-end')).toEqual([expect.objectContaining({ layerId: long.id, ranges: [[5, 6]] })]);
    expect(by('media-failed')[0]).toMatchObject({ layerId: broken.id, message: expect.stringContaining('Unsupported codec') });
    expect(by('missing-media').map((issue) => issue.layerId)).toEqual([orphan.id]);
    expect(by('audio-clipping')[0]).toMatchObject({ layerId: music.id, message: expect.stringContaining('+3.5 dBFS') });
    expect(report.issues.some((issue) => issue.layerId === hidden.id)).toBe(false);
  });

  it('notices a composition with nothing visible at all, and skips load checks without runtime media', async () => {
    const PM = editor();
    PM.proj.assets = { v: { id: 'v', name: 'v.mov', kind: 'video', dur: 10 } };
    PM.proj.layers = [PM.mkLayer('audio', { from: 0, dur: 6 }), PM.mkLayer('video', { from: 0, dur: 6, d: { asset: 'v', trim: PM.P(0), speed: PM.P(1) } })];
    PM.ProjectIndex.invalidate();
    const report = checkProject(PM);
    expect(report.mediaChecked).toBe(false);
    expect(report.issues.map((issue) => issue.code)).toEqual(['no-media']);
    PM.proj.layers = [PM.mkLayer('audio', { from: 0, dur: 6 })]; PM.ProjectIndex.invalidate();
    const empty = await PM.AgentHarness.test.handleLiveAgentTool({ runId: 'check', callId: 'c', tool: 'check_project', arguments: {}, baseRevision: 0 });
    expect(JSON.parse(empty.content[0].text).issues[0]).toMatchObject({ code: 'no-visuals', ranges: [[0, 6]] });
  });
});
