/*
 * Draws engine frames into the viewer canvas with WebGPU (S1's external
 * texture path: `importExternalTexture`, sampled 1:1).
 *
 * The canvas is `rgba16float` with extended tone mapping for `rgbaf16`
 * pools, so values above 1.0 reach an EDR display, and the preferred
 * 8-bit format for `bgra` pools. Its color space is the one the engine
 * encoded the frame for; frames are imported without conversion (they are
 * tagged BT.709/sRGB at import, and the import asks for sRGB), so the canvas
 * interprets the engine's values directly.
 *
 * WebGPU types are not in TypeScript's DOM library and the extension takes
 * no dependency for them; the few used are declared here.
 */
import type { SurfaceFormat, ViewerColorSpace } from './protocol';

/* eslint-disable @typescript-eslint/no-explicit-any */
type GPUDeviceLike = {
  queue: { submit(buffers: unknown[]): void; onSubmittedWorkDone(): Promise<void> };
  createShaderModule(d: { code: string }): unknown;
  createRenderPipeline(d: any): { getBindGroupLayout(i: number): unknown };
  createSampler(d: any): unknown;
  createBindGroup(d: any): unknown;
  createCommandEncoder(): {
    beginRenderPass(d: any): { setPipeline(p: unknown): void; setBindGroup(i: number, g: unknown): void; draw(n: number): void; end(): void };
    finish(): unknown;
  };
  importExternalTexture(d: { source: VideoFrame; colorSpace?: 'srgb' | 'display-p3' }): unknown;
  lost: Promise<{ message: string }>;
  destroy(): void;
};

type GPUCanvasContextLike = {
  configure(d: any): void;
  getCurrentTexture(): { createView(): unknown };
};

type NavigatorGpu = {
  gpu?: {
    requestAdapter(o?: any): Promise<{ requestDevice(): Promise<GPUDeviceLike> } | null>;
    getPreferredCanvasFormat(): string;
  };
};

/** What the presenter needs from a painter (WebGPU here; fakes in tests). */
export interface FramePainter {
  /** Matches the canvas to a frame's format, color space, and size. */
  configure(format: SurfaceFormat, colorSpace: ViewerColorSpace, width: number, height: number): void;
  /** Records and submits one draw of `frame`. The caller closes the frame. */
  draw(frame: VideoFrame): void;
  /** Resolves when the GPU work submitted so far completed. */
  workDone(): Promise<void>;
  dispose(): void;
}

const SHADER = /* wgsl */ `
@group(0) @binding(0) var src: texture_external;
@group(0) @binding(1) var samp: sampler;
struct Out { @builtin(position) pos: vec4f, @location(0) uv: vec2f };
@vertex fn vs(@builtin(vertex_index) i: u32) -> Out {
  let p = vec2f(f32((i << 1u) & 2u), f32(i & 2u));
  var o: Out;
  o.pos = vec4f(p * 2.0 - 1.0, 0.0, 1.0);
  o.uv = vec2f(p.x, 1.0 - p.y);
  return o;
}
@fragment fn fs(v: Out) -> @location(0) vec4f {
  return vec4f(textureSampleBaseClampToEdge(src, samp, v.uv).rgb, 1.0);
}`;

/** A page canvas, or the `OffscreenCanvas` it handed to the presentation worker. */
export type PaintTarget = HTMLCanvasElement | OffscreenCanvas;

export async function createWebGpuPainter(canvas: PaintTarget, nav: NavigatorGpu = navigator as unknown as NavigatorGpu): Promise<FramePainter> {
  const gpu = nav.gpu;
  if (!gpu) throw new Error('WebGPU is not available');
  const adapter = await gpu.requestAdapter({ powerPreference: 'high-performance' });
  if (!adapter) throw new Error('no WebGPU adapter');
  const device = await adapter.requestDevice();
  const context = (canvas as HTMLCanvasElement).getContext('webgpu') as unknown as GPUCanvasContextLike | null;
  if (!context) throw new Error('no WebGPU canvas context');
  const module = device.createShaderModule({ code: SHADER });
  const sampler = device.createSampler({ magFilter: 'nearest', minFilter: 'nearest' });
  const pipelines = new Map<string, { getBindGroupLayout(i: number): unknown }>();
  const pipelineFor = (format: string) => {
    let pipeline = pipelines.get(format);
    if (!pipeline) {
      pipeline = device.createRenderPipeline({
        layout: 'auto',
        vertex: { module, entryPoint: 'vs' },
        fragment: { module, entryPoint: 'fs', targets: [{ format }] },
        primitive: { topology: 'triangle-list' }
      });
      pipelines.set(format, pipeline);
    }
    return pipeline;
  };
  let canvasFormat = gpu.getPreferredCanvasFormat();
  let configured = '';
  return {
    configure(format, colorSpace, width, height) {
      const f16 = format === 'rgba16Float';
      canvasFormat = f16 ? 'rgba16float' : gpu.getPreferredCanvasFormat();
      const key = `${canvasFormat}:${colorSpace}:${width}x${height}`;
      if (key === configured) return;
      configured = key;
      if (canvas.width !== width) canvas.width = width;
      if (canvas.height !== height) canvas.height = height;
      context.configure({
        device,
        format: canvasFormat,
        alphaMode: 'opaque',
        colorSpace: colorSpace === 'displayP3' ? 'display-p3' : 'srgb',
        toneMapping: { mode: f16 ? 'extended' : 'standard' },
        usage: 0x10 | 0x01 // RENDER_ATTACHMENT | COPY_SRC
      });
      pipelineFor(canvasFormat);
    },
    draw(frame) {
      const pipeline = pipelineFor(canvasFormat);
      const external = device.importExternalTexture({ source: frame, colorSpace: 'srgb' });
      const group = device.createBindGroup({
        layout: pipeline.getBindGroupLayout(0),
        entries: [
          { binding: 0, resource: external },
          { binding: 1, resource: sampler }
        ]
      });
      const encoder = device.createCommandEncoder();
      const pass = encoder.beginRenderPass({
        colorAttachments: [{ view: context.getCurrentTexture().createView(), loadOp: 'clear', storeOp: 'store', clearValue: { r: 0, g: 0, b: 0, a: 1 } }]
      });
      pass.setPipeline(pipeline);
      pass.setBindGroup(0, group);
      pass.draw(3);
      pass.end();
      device.queue.submit([encoder.finish()]);
    },
    workDone: () => device.queue.onSubmittedWorkDone(),
    dispose: () => device.destroy()
  };
}
