// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { makePM } from '../../__tests__/make-pm';
import { figmaNode } from './__fixtures__/clipboard';
import { pasteMedia } from '../media-paste';
import { runFigmaPaste } from './paste';

const read = vi.hoisted(() => vi.fn());
vi.mock('./decode', () => ({ readFigmaSelection: read }));
beforeEach(() => { read.mockReset(); read.mockResolvedValue({ nodes: [figmaNode(1)], blobs: [] }); });
afterEach(() => document.body.replaceChildren());

function editor() {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
  PM.proj = PM.mkProject({ w: 800, h: 600, dur: 8 }); PM.time = 2;
  PM.confirm = vi.fn(async () => true); PM.toast = vi.fn(); PM.importFiles = vi.fn();
  PM.assets = { importBatch: vi.fn(async () => {
    const asset = { id: 'preview', w: 200, h: 120 };
    PM.proj.assets.preview = asset;
    return [{ status: 'created', asset }];
  }) };
  PM.commandForAsset = vi.fn((id, from) => ({ type: 'add_layer', layerType: 'image', from, content: { asset: id } }));
  const source = { html: '(figma)copied-data(/figma)', image: new File(['preview'], 'preview.png', { type: 'image/png' }),
    project: PM.proj, generation: PM.projGeneration, compId: PM.proj.compId, at: PM.time };
  return { PM, source };
}

describe('native Figma paste', () => {
  it('claims Figma before its PNG preview, imports editable layers and supports one undo', async () => {
    const { PM, source } = editor();
    const event = new ClipboardEvent('paste', { cancelable: true });
    Object.defineProperty(event, 'clipboardData', { value: { files: [source.image], getData: () => source.html } });
    expect(pasteMedia(PM, event)).toBe(true); expect(event.defaultPrevented).toBe(true);
    await vi.waitFor(() => expect(PM.proj.layers).toHaveLength(1));
    expect(PM.proj.layers[0].type).toBe('shape');
    expect(PM.confirm).not.toHaveBeenCalled(); expect(PM.importFiles).not.toHaveBeenCalled();
    expect(PM.hist.undo()).toBe(true); expect(PM.proj.layers).toHaveLength(0);
  });
  it('explains the fallback and loss of editability before importing an unsupported selection', async () => {
    const { PM, source } = editor();
    read.mockResolvedValue({ nodes: [figmaNode(1, 'RECTANGLE', { effects: [{ type: 'DROP_SHADOW' }] })], blobs: [] });
    PM.confirm.mockImplementation(async (options: any) => {
      expect(options.detail).toContain('effects'); expect(options.detail).toContain('entire selection');
      expect(options.detail).toContain('individual parts will not be editable');
      expect(PM.assets.importBatch).not.toHaveBeenCalled(); expect(PM.proj.layers).toHaveLength(0);
      return true;
    });
    await runFigmaPaste(PM, source, read);
    expect(PM.proj.layers[0]).toMatchObject({ type: 'image', from: 2, name: 'Figma selection (image fallback)', d: { asset: 'preview', w: 100, h: 60 } });
    expect(PM.hist.undo()).toBe(true); expect(PM.proj.layers).toHaveLength(0);
    expect(PM.hist.redo()).toBe(true); expect(PM.proj.layers).toHaveLength(1);
  });
  it('cancels fallback without importing assets or changing source/history', async () => {
    const { PM, source } = editor(); PM.confirm.mockResolvedValue(false);
    read.mockResolvedValue({ nodes: [figmaNode(1, 'INSTANCE')], blobs: [] });
    const before = PM.serialize();
    await runFigmaPaste(PM, source, read);
    expect(PM.serialize()).toBe(before); expect(PM.assets.importBatch).not.toHaveBeenCalled(); expect(PM.hist.undo()).toBe(false);
  });
  it('provides PNG instructions when the clipboard has no fallback image', async () => {
    const { PM, source } = editor();
    read.mockResolvedValue({ nodes: [figmaNode(1, 'INSTANCE')], blobs: [] });
    await runFigmaPaste(PM, { ...source, image: undefined }, read);
    expect(PM.confirm.mock.calls[0][0].detail).toContain('Copy as PNG');
    expect(PM.assets.importBatch).not.toHaveBeenCalled(); expect(PM.proj.layers).toHaveLength(0);
  });
  it('offers image fallback for a newer or damaged clipboard format', async () => {
    const { PM, source } = editor(); read.mockRejectedValue(new Error('Unsupported Figma format'));
    await runFigmaPaste(PM, source, read);
    expect(PM.confirm.mock.calls[0][0].detail).toContain('Unsupported Figma format');
    expect(PM.proj.layers[0].type).toBe('image');
  });
  it.each(['project', 'composition', 'undo'] as const)('does not paste into another destination after %s changes during decoding', async kind => {
    const { PM, source } = editor();
    await runFigmaPaste(PM, source, async () => {
      if (kind === 'project') PM.replaceProject(PM.mkProject());
      if (kind === 'composition') PM.proj.compId = 'different';
      if (kind === 'undo') PM.projGeneration++;
      return { nodes: [figmaNode(1)], blobs: [] };
    });
    expect(PM.proj.layers).toHaveLength(0); expect(PM.confirm).not.toHaveBeenCalled();
  });
  it('rechecks the destination after the fallback prompt', async () => {
    const { PM, source } = editor(); read.mockResolvedValue({ nodes: [figmaNode(1, 'INSTANCE')], blobs: [] });
    PM.confirm.mockImplementation(async () => { PM.replaceProject(PM.mkProject()); return true; });
    await runFigmaPaste(PM, source, read);
    expect(PM.proj.layers).toHaveLength(0); expect(PM.assets.importBatch).not.toHaveBeenCalled();
  });
  it.each(['input', 'textarea', 'editable', 'transcript'])('leaves Figma paste inside a %s alone', kind => {
    const { PM, source } = editor();
    const target = document.createElement(['input', 'textarea'].includes(kind) ? kind : 'div');
    if (kind === 'editable') target.contentEditable = 'true';
    if (kind === 'transcript') target.dataset.nativeText = '';
    document.body.append(target);
    const event = new ClipboardEvent('paste', { cancelable: true });
    Object.defineProperty(event, 'clipboardData', { value: { files: [source.image], getData: () => source.html } });
    vi.spyOn(event, 'composedPath').mockReturnValue([target, document.body, window]);
    expect(pasteMedia(PM, event)).toBe(false); expect(event.defaultPrevented).toBe(false);
    expect(read).not.toHaveBeenCalled();
  });
});
