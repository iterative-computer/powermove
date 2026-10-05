// @ts-nocheck -- drives the legacy PM registry like the command suites.
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { makePM } from '../legacy/__tests__/make-pm';
import { installCaptions } from './install';

vi.mock('../transcription/client', () => ({ transcriptionStatus: vi.fn(), ensureTranscriptionModel: vi.fn(), transcribe: vi.fn() }));
vi.mock('../media/media-path', () => ({ resolveMediaPath: vi.fn() }));

/* A cue selection belongs to one captions layer selected on its own; any
   layer-level selection ends it, so Delete acts on what the user sees. */

let PM: any;
let captions: any, other: any;

beforeEach(() => {
  PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing', 'gl/shaders');
  PM.proj = PM.mkProject({ name: 'Selection', w: 1920, h: 1080, fps: 30, dur: 6 });
  PM.Kernel.services.register('shaderHooks', { syncShaderUniforms() {} });
  other = PM.mkLayer('solid', { name: 'Background', from: 0, dur: 6 });
  PM.proj.layers.push(other);
  installCaptions(PM);
  PM.Edit.apply({ type: 'add_captions', name: 'Subs', cues: [{ start: 0, end: 1, text: 'One' }, { start: 1, end: 2, text: 'Two' }] });
  captions = PM.proj.layers.find((layer: any) => layer.type === 'captions');
});

describe('cue selection', () => {
  it('selects the captions layer alone and ends when another layer joins the selection', () => {
    PM.selectLayers([captions.id, other.id]);
    PM.Captions.select(captions.id, [captions.d.cues[0].id]);
    expect(PM.sel.layers).toEqual([captions.id]);
    PM.selectLayers([captions.id, other.id]);
    expect(PM.Captions.selection()).toEqual({ layerId: null, cues: [] });
    expect(PM.Captions.deleteSelectedCues()).toBe(false);
  });

  it('deletes only selected cues while the layer is the only selection', () => {
    PM.Captions.select(captions.id, [captions.d.cues[0].id]);
    expect(PM.Captions.deleteSelectedCues()?.ok).toBe(true);
    expect(captions.d.cues.map((cue: any) => cue.text)).toEqual(['Two']);
    expect(PM.L(captions.id)).toBeTruthy();
  });

  it('clears when the layer is deselected', () => {
    PM.Captions.select(captions.id, [captions.d.cues[1].id]);
    PM.selectLayers([other.id]);
    expect(PM.Captions.selection().layerId).toBeNull();
  });
});
