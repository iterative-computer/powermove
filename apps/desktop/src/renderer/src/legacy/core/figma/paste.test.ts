// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { makePM } from '../../__tests__/make-pm';
import { figmaNode } from './__fixtures__/clipboard';
import { pasteMedia } from '../media-paste';
import { clipboardDoc } from './adapter';
import { receiveFigma, runFigmaPaste } from './paste';

const read = vi.hoisted(() => vi.fn());
vi.mock('./decode', () => ({ readFigmaDoc: async (html: string) => clipboardDoc(await read(html)) }));
beforeEach(() => {
  read.mockReset();
  read.mockResolvedValue({ nodes: [figmaNode(1)], blobs: [] });
});
afterEach(() => document.body.replaceChildren());

function editor() {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
  PM.proj = PM.mkProject({ w: 800, h: 600, dur: 8 });
  PM.time = 2;
  PM.confirm = vi.fn(async () => true);
  PM.toast = vi.fn();
  PM.importFiles = vi.fn();
  PM.assets = {
    importBatch: vi.fn(async () => {
      const asset = { id: 'preview', w: 200, h: 120 };
      PM.proj.assets.preview = asset;
      return [{ status: 'created', asset }];
    }),
  };
  PM.commandForAsset = vi.fn((id, from) => ({ type: 'add_layer', layerType: 'image', from, content: { asset: id } }));
  const source = {
    html: '(figma)copied-data(/figma)',
    image: new File(['preview'], 'preview.png', { type: 'image/png' }),
    project: PM.proj,
    generation: PM.projGeneration,
    compId: PM.proj.compId,
    at: PM.time,
  };
  return { PM, source };
}

describe('native Figma paste', () => {
  it('claims Figma before its PNG preview, imports editable layers and supports one undo', async () => {
    const { PM, source } = editor();
    const event = new ClipboardEvent('paste', { cancelable: true });
    Object.defineProperty(event, 'clipboardData', { value: { files: [source.image], getData: () => source.html } });
    expect(pasteMedia(PM, event)).toBe(true);
    expect(event.defaultPrevented).toBe(true);
    await vi.waitFor(() => expect(PM.proj.layers).toHaveLength(1));
    expect(PM.proj.layers[0].type).toBe('shape');
    expect(PM.confirm).not.toHaveBeenCalled();
    expect(PM.importFiles).not.toHaveBeenCalled();
    expect(PM.hist.undo()).toBe(true);
    expect(PM.proj.layers).toHaveLength(0);
  });
  it('explains a missing per-node preview without blocking or mutating the project', async () => {
    const { PM, source } = editor();
    read.mockResolvedValue({ nodes: [figmaNode(1, 'RECTANGLE', { effects: [{ type: 'GLASS' } as any] })], blobs: [] });
    await runFigmaPaste(PM, { ...source, image: undefined }, read);
    expect(PM.confirm).not.toHaveBeenCalled();
    expect(PM.toast.mock.calls[0][0]).toContain('Send to Powermove');
    expect(PM.proj.layers).toHaveLength(0);
  });
  it('reports damaged clipboard data without importing unrelated pixels', async () => {
    const { PM, source } = editor();
    read.mockRejectedValue(new Error('Unsupported Figma format'));
    await runFigmaPaste(PM, source, read);
    expect(PM.toast.mock.calls[0][0]).toContain('Unsupported Figma format');
    expect(PM.proj.layers).toHaveLength(0);
  });
  it.each(['project', 'composition', 'undo'] as const)(
    'does not paste into another destination after %s changes during decoding',
    async (kind) => {
      const { PM, source } = editor();
      await runFigmaPaste(PM, source, async () => {
        if (kind === 'project') PM.replaceProject(PM.mkProject());
        if (kind === 'composition') PM.proj.compId = 'different';
        if (kind === 'undo') PM.projGeneration++;
        return { nodes: [figmaNode(1)], blobs: [] };
      });
      expect(PM.proj.layers).toHaveLength(0);
      expect(PM.confirm).not.toHaveBeenCalled();
    },
  );
  it.each(['input', 'textarea', 'editable', 'transcript'])('leaves Figma paste inside a %s alone', (kind) => {
    const { PM, source } = editor();
    const target = document.createElement(['input', 'textarea'].includes(kind) ? kind : 'div');
    if (kind === 'editable') target.contentEditable = 'true';
    if (kind === 'transcript') target.dataset.nativeText = '';
    document.body.append(target);
    const event = new ClipboardEvent('paste', { cancelable: true });
    Object.defineProperty(event, 'clipboardData', { value: { files: [source.image], getData: () => source.html } });
    vi.spyOn(event, 'composedPath').mockReturnValue([target, document.body, window]);
    expect(pasteMedia(PM, event)).toBe(false);
    expect(event.defaultPrevented).toBe(false);
    expect(read).not.toHaveBeenCalled();
  });
});

it('clears the previous import notice before a re-sync decision and only announces committed changes', async () => {
  const { PM } = editor();
  PM.dismissToast = vi.fn();
  const doc = clipboardDoc({ nodes: [figmaNode(1)], blobs: [] });
  await receiveFigma(PM, doc);
  expect(PM.toast).toHaveBeenLastCalledWith('Imported 1 layer', 8000, { key: 'figma-import', kind: 'status' });
  PM.modal = vi.fn();
  const cancelled = receiveFigma(PM, doc);
  expect(PM.dismissToast).toHaveBeenLastCalledWith('figma-import');
  expect(PM.toast).toHaveBeenCalledTimes(1);
  PM.modal.mock.calls[0][0].onClose();
  await expect(cancelled).rejects.toThrow('cancelled');
  expect(PM.toast).toHaveBeenCalledTimes(1);
  const updated = receiveFigma(PM, doc);
  expect(PM.toast).toHaveBeenCalledTimes(1);
  PM.modal.mock.calls[1][0].actions[1].run();
  await updated;
  expect(PM.toast).toHaveBeenLastCalledWith('Updated 1 layer', 8000, { key: 'figma-import', kind: 'status' });
});
