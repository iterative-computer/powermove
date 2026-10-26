import { createHash } from 'node:crypto';
import { expect, it, vi } from 'vitest';
import { saveIncrementalProject } from './project-file';

const bytes = new TextEncoder().encode('owned media');
const sha256 = createHash('sha256').update(bytes).digest('hex');
function setup() {
  const source = { size: bytes.length, type: 'video/mp4', sha256, read: vi.fn(async () => bytes) };
  const store = { get: vi.fn(), put: vi.fn(), getForSave: vi.fn(async () => ({ source, revision: 'v1' })) };
  const bridge = { begin: vi.fn(async () => ({ ok: true as const, token: 'save', required: ['owned'] })),
    chunk: vi.fn(async () => {}), finish: vi.fn(async () => ({ ok: true as const, path: 'file.pmv' })), abort: vi.fn(async () => {}) };
  const document = { proj: { assets: { owned: { id: 'owned', name: 'Synthetic.mp4' } } } };
  return { source, store, bridge, save: (signal?: AbortSignal) => saveIncrementalProject(document, JSON.stringify(document), store, bridge,
    { name: 'file.pmv', projectId: 'one', saveAs: false }, () => {}, signal) };
}

it('begins the native transaction before reading owned media, without creating complete Blobs', async () => {
  const { source, bridge, save, store } = setup();
  source.read.mockImplementation(async () => { expect(bridge.begin).toHaveBeenCalledOnce(); return bytes; });
  expect(await save()).toMatchObject({ ok: true });
  expect(source.read).toHaveBeenCalledWith(0, bytes.length, undefined);
  expect(store.get).not.toHaveBeenCalled(); expect(bridge.finish).toHaveBeenCalledOnce();
});

it.each(['corrupt', 'truncated', 'transport'])('aborts on %s source and permits a clean retry', async failure => {
  const { source, bridge, save } = setup();
  source.read.mockImplementationOnce(async () => {
    if (failure === 'transport') throw new TypeError('Failed to fetch');
    return failure === 'corrupt' ? new Uint8Array(bytes.length) : bytes.subarray(1);
  });
  await expect(save()).rejects.toThrow('Synthetic.mp4');
  expect(bridge.finish).not.toHaveBeenCalled(); expect(bridge.abort).toHaveBeenCalledWith('save');
  expect(await save()).toMatchObject({ ok: true });
});

it('propagates cancellation to an active source, aborts the native transaction and never reports completion', async () => {
  const { source, bridge, save } = setup();
  const controller = new AbortController();
  source.read.mockImplementationOnce(async (_offset?: number, _length?: number, signal?: AbortSignal) => {
    expect(signal).toBe(controller.signal); controller.abort(); signal!.throwIfAborted(); return bytes;
  });
  await expect(save(controller.signal)).rejects.toMatchObject({ name: 'AbortError' });
  expect(bridge.finish).not.toHaveBeenCalled(); expect(bridge.abort).toHaveBeenCalledOnce();
});

it('does not consume media that the main process can reuse', async () => {
  const { source, bridge, save } = setup(); bridge.begin.mockResolvedValueOnce({ ok: true, token: 'save', required: [] });
  expect(await save()).toMatchObject({ ok: true }); expect(source.read).not.toHaveBeenCalled();
});
