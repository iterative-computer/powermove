import { describe, expect, it } from 'vitest';
import { createWebGpuPainter } from './gpu';

function fixture(fail = false) {
  const compiled: string[] = [];
  let destroyed = false;
  const device = {
    queue: { submit() {}, onSubmittedWorkDone: async () => {} },
    createShaderModule: () => ({}), createSampler: () => ({}),
    createRenderPipeline() { throw new Error('compile must be asynchronous'); },
    async createRenderPipelineAsync(d: any) {
      compiled.push(d.fragment.targets[0].format);
      if (fail) throw new Error('pipeline compile failed');
      return { getBindGroupLayout: () => ({}) };
    },
    createBindGroup: () => ({}), createCommandEncoder: () => ({} as any), importExternalTexture: () => ({}),
    lost: new Promise<{ message: string }>(() => {}), destroy() { destroyed = true; }
  };
  const nav = { gpu: { requestAdapter: async () => ({ requestDevice: async () => device }), getPreferredCanvasFormat: () => 'bgra8unorm' } };
  const canvas = { width: 1, height: 1, getContext: () => ({ configure() {}, getCurrentTexture: () => ({ createView: () => ({}) }) }) } as unknown as OffscreenCanvas;
  return { nav, canvas, compiled, destroyed: () => destroyed };
}

describe('GPU painter warmup', () => {
  it('compiles SDR and EDR pipelines before the painter is handed to a session', async () => {
    const f = fixture();
    const painter = await createWebGpuPainter(f.canvas, f.nav);
    expect(f.compiled.sort()).toEqual(['bgra8unorm', 'rgba16float']);
    painter.configure('bgra8Unorm', 'srgb', 640, 360);
    painter.configure('rgba16Float', 'displayP3', 1920, 1080);
    expect(f.compiled.length).toBe(2);
    painter.dispose(); expect(f.destroyed()).toBe(true);
  });

  it('destroys the device if warmup fails', async () => {
    const f = fixture(true);
    await expect(createWebGpuPainter(f.canvas, f.nav)).rejects.toThrow('pipeline compile failed');
    expect(f.destroyed()).toBe(true);
  });
});
