import { createFramePainter } from '../../src/extensions/viewer/engine-present/painter';
import { createWebGpuPainter } from '../../src/extensions/viewer/engine-present/gpu';

const half = (h: number): number => {
  const sign = h >> 15 ? -1 : 1, exponent = (h >> 10) & 31, fraction = h & 1023;
  return sign * (exponent === 0 ? fraction * 2 ** -24 : exponent === 31 ? Infinity : (1 + fraction / 1024) * 2 ** (exponent - 15));
};

/** Read both actual F16 attachments, normalizing WebGL's bottom-up rows. */
export async function run() {
  const results: { space: string; color: number[]; actual: number[]; expected: number[] }[] = [];
  const source = new OffscreenCanvas(2, 2);
  const gl = source.getContext('webgl2', { alpha: true, premultipliedAlpha: true, antialias: false, preserveDrawingBuffer: true })!;
  gl.getExtension('EXT_color_buffer_float');
  (gl as any).drawingBufferStorage(gl.RGBA16F, 2, 2);
  gl.drawingBufferColorSpace = 'srgb';
  try {
    for (const space of ['srgb', 'display-p3'] as const) {
      for (const color of [[2, .25, .5, 1], [.1, .2, .3, .5], [0, 0, 0, 0], [.5, .125, 1, 1], [2, .5, 1, .25], [.5, .25, .125, 0], [-.5, .125, 1, 1]]) {
        gl.clearColor(color[0]!, color[1]!, color[2]!, color[3]!); gl.clear(gl.COLOR_BUFFER_BIT);
        // An asymmetric corner proves orientation independently of color.
        gl.enable(gl.SCISSOR_TEST); gl.scissor(0, 0, 1, 1);
        gl.clearColor(.75, .0625, .5, 1); gl.clear(gl.COLOR_BUFFER_BIT); gl.disable(gl.SCISSOR_TEST);
        const frame = new VideoFrame(source, { timestamp: 0 });
        const values: number[][] = [];
        try {
          for (const kind of ['native', 'reference'] as const) {
            const canvas = new OffscreenCanvas(2, 2);
            let device: any;
            const gpu = (navigator as any).gpu;
            const nav = { gpu: { getPreferredCanvasFormat: () => gpu.getPreferredCanvasFormat(), requestAdapter: async (options: unknown) => {
              const adapter = await gpu.requestAdapter(options);
              return { requestDevice: async () => device = await adapter.requestDevice() };
            } } };
            const painter = kind === 'native' ? await createFramePainter(canvas) : await createWebGpuPainter(canvas, nav);
            try {
              painter.configure('rgba16Float', space === 'srgb' ? 'srgb' : 'displayP3', 2, 2);
              painter.draw(frame);
              if (kind === 'native') {
                const context = canvas.getContext('webgl2')!;
                const out = new Float32Array(16);
                context.readPixels(0, 0, 2, 2, context.RGBA, context.FLOAT, out);
                if (context.getError() !== context.NO_ERROR) throw new Error('F16 readPixels failed');
                values.push([...out.slice(8), ...out.slice(0, 8)]);
              } else {
                const texture = (canvas.getContext('webgpu') as any).getCurrentTexture();
                const buffer = device.createBuffer({ size: 512, usage: 1 | 8 });
                try {
                  const encoder = device.createCommandEncoder();
                  encoder.copyTextureToBuffer({ texture }, { buffer, bytesPerRow: 256 }, [2, 2]);
                  device.queue.submit([encoder.finish()]); await buffer.mapAsync(1);
                  const raw = new Uint16Array(buffer.getMappedRange());
                  values.push([...raw.slice(0, 8), ...raw.slice(128, 136)].map(half));
                  buffer.unmap();
                } finally { buffer.destroy(); }
              }
              await painter.workDone();
            } finally { painter.dispose(); }
          }
        } finally { frame.close(); }
        results.push({ space, color, actual: values[0]!, expected: values[1]! });
      }
    }
  } finally { gl.getExtension('WEBGL_lose_context')?.loseContext(); }
  return results;
}
