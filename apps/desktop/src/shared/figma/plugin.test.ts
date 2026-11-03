import { afterEach, it, expect, vi } from 'vitest';
afterEach(() => vi.unstubAllGlobals());
it('serializes official API data, resolves instances, dedupes image bytes and exports only unsupported nodes', async () => {
  const image = {
    type: 'IMAGE',
    imageHash: 'synthetic-image',
    scaleMode: 'FILL',
    visible: true,
    opacity: 1,
    blendMode: 'NORMAL',
  };
  const make = (id: string, patch: any = {}) => ({
    id,
    type: 'RECTANGLE',
    name: id,
    width: 100,
    height: 60,
    absoluteTransform: [
      [1, 0, 0],
      [0, 1, 0],
    ],
    fills: [{ type: 'SOLID', color: { r: 1, g: 0, b: 0 } }],
    strokes: [],
    effects: [],
    visible: true,
    opacity: 1,
    blendMode: 'NORMAL',
    absoluteRenderBounds: { x: 0, y: 0, width: 100, height: 60 },
    exportAsync: vi.fn(async () => Uint8Array.from([137, 80, 78, 71, 13, 10, 26, 10])),
    ...patch,
  });
  const editable = make('editable'),
    first = make('image-a', { fills: [image] }),
    second = make('image-b', { fills: [image] });
  const instance = make('instance', { type: 'INSTANCE', children: [editable, first, second] });
  const page = { id: 'page', parent: null, children: [instance], selection: [instance] };
  instance.parent = page;
  for (const n of instance.children) n.parent = instance;
  const bytes = vi.fn(async () => Uint8Array.from([1, 2, 3]));
  vi.stubGlobal('__html__', '');
  vi.stubGlobal('figma', {
    showUI: vi.fn(),
    fileKey: 'synthetic-file',
    currentPage: page,
    ui: { postMessage: vi.fn() },
    clientStorage: { getAsync: async () => null },
    getImageByHash: () => ({ getBytesAsync: bytes }),
    base64Encode: (b: Uint8Array) => Buffer.from(b).toString('base64'),
  });
  const pluginPath = new URL('../../../../../packages/figma-plugin/code.ts', import.meta.url).pathname;
  const { serializeSelection } = await import(pluginPath);
  const doc = await serializeSelection();
  expect(doc.nodes[0]!.children).toHaveLength(3);
  expect(doc.images).toEqual({ 'synthetic-image': 'AQID' });
  expect(bytes).toHaveBeenCalledTimes(1);
  expect(editable.exportAsync).not.toHaveBeenCalled();
  expect(instance.exportAsync).not.toHaveBeenCalled();
  expect(first.exportAsync).toHaveBeenCalledWith({ format: 'PNG', constraint: { type: 'SCALE', value: 2 } });
  expect(second.exportAsync).toHaveBeenCalledTimes(1);
});
