/*
 * Runs the WebGL2 F16 painter (production) and the WebGPU external-texture
 * painter (the earlier production path) on the viewer's own surface
 * imports: VideoFrames of IOSurfaces the engine rendered, imported by main
 * and sent through Electron's sharedTexture bridge. Compares both
 * painters' attachments value for value and reports the frames' alpha
 * classes. Bundled into the page by engine-frame-painter.spec.ts.
 */
import { createFramePainter } from '../../src/extensions/viewer/engine-present/painter';
import { createWebGpuPainter } from '../../src/extensions/viewer/engine-present/gpu';
import type { SurfaceFormat, ViewerColorSpace } from '../../src/extensions/viewer/engine-present/protocol';

const half = (h: number): number => {
  const sign = h >> 15 ? -1 : 1, exponent = (h >> 10) & 31, fraction = h & 1023;
  return sign * (exponent === 0 ? fraction * 2 ** -24 : exponent === 31 ? Infinity : (1 + fraction / 1024) * 2 ** (exponent - 15));
};

export type SlotResult = {
  poolIndex: number;
  videoFrameFormat: string | null;
  width: number;
  height: number;
  /** Pixels whose values differ between the painters (after 8-bit quantization for a bgra pool). */
  differing: number;
  maxDifference: number;
  firstDifference: { x: number; y: number; webgl: number[]; webgpu: number[] } | null;
  /** Pixels by alpha (from the WebGPU attachment): 0, strictly between, 1. */
  alpha: { zero: number; partial: number; one: number };
  /** Distinct opaque colors seen (capped), so the frame is known to hold the composition. */
  colors: number;
};

/** Paints every held slot of the current pool with both painters. */
export async function run(): Promise<{ generation: number; format: SurfaceFormat; colorSpace: ViewerColorSpace; edr: boolean; slots: SlotResult[] }> {
  const PM = (window as any).PM;
  const presentation = PM.Kernel.services.get('viewer').enginePresentation;
  const service = PM.Kernel.services.get('engine.viewer');
  const generation: number = presentation.stats().generation;
  const state = presentation.state;
  const colorSpace: ViewerColorSpace = state.colorSpace;
  const slots: SlotResult[] = [];
  let poolFormat: SurfaceFormat = 'rgba16Float';
  for (let poolIndex = 0; poolIndex < 64; poolIndex++) {
    const frame: VideoFrame | null = service.source.videoFrame(generation, poolIndex);
    if (!frame) continue;
    try {
      const width = frame.codedWidth, height = frame.codedHeight;
      const format: SurfaceFormat = frame.format === 'BGRA' || frame.format === 'BGRX' ? 'bgra8Unorm' : 'rgba16Float';
      poolFormat = format;
      // WebGL2 F16 painter: read its RGBA16F drawing buffer as floats (bottom-up rows).
      const glCanvas = new OffscreenCanvas(width, height);
      const glPainter = await createFramePainter(glCanvas);
      let webgl: Float32Array;
      try {
        glPainter.configure(format, colorSpace, width, height);
        glPainter.draw(frame);
        const context = glCanvas.getContext('webgl2')!;
        webgl = new Float32Array(width * height * 4);
        context.readPixels(0, 0, width, height, context.RGBA, context.FLOAT, webgl);
        if (context.getError() !== context.NO_ERROR) throw new Error('F16 readPixels failed');
        await glPainter.workDone();
      } finally { glPainter.dispose(); }
      // WebGPU painter: copy its current canvas texture (top-down rows).
      const gpuCanvas = new OffscreenCanvas(width, height);
      let device: any;
      const gpu = (navigator as any).gpu;
      const nav = { gpu: { getPreferredCanvasFormat: () => gpu.getPreferredCanvasFormat(), requestAdapter: async (options: unknown) => {
        const adapter = await gpu.requestAdapter(options);
        return { requestDevice: async () => device = await adapter.requestDevice() };
      } } };
      const gpuPainter = await createWebGpuPainter(gpuCanvas, nav);
      let webgpu: (x: number, y: number) => number[];
      let raw: Uint16Array | Uint8Array;
      let bytesPerRow: number;
      const f16 = format === 'rgba16Float';
      const canvasFormat: string = f16 ? 'rgba16float' : gpu.getPreferredCanvasFormat();
      try {
        gpuPainter.configure(format, colorSpace, width, height);
        gpuPainter.draw(frame);
        const texture = (gpuCanvas.getContext('webgpu') as any).getCurrentTexture();
        const pixelBytes = f16 ? 8 : 4;
        bytesPerRow = Math.ceil((width * pixelBytes) / 256) * 256;
        const buffer = device.createBuffer({ size: bytesPerRow * height, usage: 1 | 8 });
        try {
          const encoder = device.createCommandEncoder();
          encoder.copyTextureToBuffer({ texture }, { buffer, bytesPerRow }, [width, height]);
          device.queue.submit([encoder.finish()]);
          await buffer.mapAsync(1);
          raw = f16 ? new Uint16Array(buffer.getMappedRange().slice(0)) : new Uint8Array(buffer.getMappedRange().slice(0));
          buffer.unmap();
        } finally { buffer.destroy(); }
        await gpuPainter.workDone();
      } finally { gpuPainter.dispose(); }
      const bgra = canvasFormat === 'bgra8unorm';
      webgpu = (x, y) => {
        if (f16) {
          const at = (y * bytesPerRow) / 2 + x * 4;
          return [half(raw[at]!), half(raw[at + 1]!), half(raw[at + 2]!), half(raw[at + 3]!)];
        }
        const at = y * bytesPerRow + x * 4;
        const v = [raw[at]!, raw[at + 1]!, raw[at + 2]!, raw[at + 3]!];
        return bgra ? [v[2]!, v[1]!, v[0]!, v[3]!] : v;
      };
      const result: SlotResult = { poolIndex, videoFrameFormat: frame.format, width, height, differing: 0, maxDifference: 0, firstDifference: null, alpha: { zero: 0, partial: 0, one: 0 }, colors: 0 };
      const colors = new Set<string>();
      for (let y = 0; y < height; y++) {
        const glRow = height - 1 - y;
        for (let x = 0; x < width; x++) {
          const at = (glRow * width + x) * 4;
          const gl = [webgl[at]!, webgl[at + 1]!, webgl[at + 2]!, webgl[at + 3]!];
          // An 8-bit pool shows its bytes: the F16 buffer must hold each byte's value.
          const ours = f16 ? gl : gl.map((v) => Math.round(v * 255));
          const theirs = webgpu(x, y);
          let max = 0;
          for (let c = 0; c < 4; c++) max = Math.max(max, Math.abs(ours[c]! - theirs[c]!));
          if (max > 0) {
            result.differing++;
            result.maxDifference = Math.max(result.maxDifference, max);
            result.firstDifference ??= { x, y, webgl: ours, webgpu: theirs };
          }
          const a = f16 ? theirs[3]! : theirs[3]! / 255;
          if (a === 0) result.alpha.zero++;
          else if (a === 1) result.alpha.one++;
          else result.alpha.partial++;
          if (a === 1 && colors.size < 64) colors.add(theirs.slice(0, 3).join(','));
        }
      }
      result.colors = colors.size;
      slots.push(result);
    } finally { frame.close(); }
  }
  return { generation, format: poolFormat, colorSpace, edr: state.edr, slots };
}
