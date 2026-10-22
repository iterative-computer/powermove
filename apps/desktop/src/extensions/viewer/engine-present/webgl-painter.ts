/*
 * F16 presentation for Electron 44. VideoFrame uploads here do
 * not call Blink's WebGPU external-texture wrapper cache. The capability checks
 * deliberately reject an 8-bit drawing buffer instead of silently losing EDR.
 * The engine still renders every frame natively; this only presents its surface.
 */
import { DeviceLostError, type FramePainter, type PaintTarget } from './gpu';
import type { SurfaceFormat, ViewerColorSpace } from './protocol';

type GlWithFloatDrawingBuffer = WebGL2RenderingContext & {
  drawingBufferStorage?: (format: number, width: number, height: number) => void;
  drawingBufferFormat?: number;
  drawingBufferColorSpace: 'srgb' | 'display-p3';
  unpackColorSpace: 'srgb' | 'display-p3';
};

export class WebGlPainterCapabilityError extends Error {
  constructor(message: string) {
    super(`WebGL2 F16 presentation is unavailable: ${message}`);
    this.name = 'WebGlPainterCapabilityError';
  }
}

const VERTEX = `#version 300 es
out vec2 v_uv;
void main() {
  vec2 p = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2));
  gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
  v_uv = vec2(p.x, 1.0 - p.y);
}`;

const FRAGMENT = `#version 300 es
precision highp float;
in vec2 v_uv;
uniform sampler2D u_frame;
out vec4 color;
void main() { color = texture(u_frame, v_uv); }
`;

function shader(gl: GlWithFloatDrawingBuffer, type: number, source: string): WebGLShader {
  const result = gl.createShader(type);
  if (!result) throw new WebGlPainterCapabilityError('shader allocation failed');
  gl.shaderSource(result, source);
  gl.compileShader(result);
  if (!gl.getShaderParameter(result, gl.COMPILE_STATUS)) {
    const reason = gl.getShaderInfoLog(result) || 'unknown shader compile error';
    gl.deleteShader(result);
    throw new WebGlPainterCapabilityError(reason);
  }
  return result;
}

type Fence = { sync: WebGLSync; timer: ReturnType<typeof setTimeout> | null; resolve: () => void; reject: (reason: Error) => void };

export async function createWebGlPainter(canvas: PaintTarget): Promise<FramePainter> {
  const gl = canvas.getContext('webgl2', {
    alpha: true,
    premultipliedAlpha: true,
    antialias: false,
    preserveDrawingBuffer: false,
    powerPreference: 'high-performance'
  }) as GlWithFloatDrawingBuffer | null;
  if (!gl) throw new WebGlPainterCapabilityError('WebGL2 context creation failed');
  if (typeof gl.drawingBufferStorage !== 'function' || !('drawingBufferColorSpace' in gl)) {
    throw new WebGlPainterCapabilityError('F16 drawing-buffer or color-space API is missing');
  }
  const attributes = gl.getContextAttributes();
  if (!attributes?.alpha || !attributes.premultipliedAlpha || attributes.antialias) {
    throw new WebGlPainterCapabilityError('required alpha, premultiplication, or antialias configuration was not accepted');
  }
  if (!gl.getExtension('EXT_color_buffer_float') && !gl.getExtension('EXT_color_buffer_half_float')) {
    throw new WebGlPainterCapabilityError('float color-buffer extension is missing');
  }

  const vertex = shader(gl, gl.VERTEX_SHADER, VERTEX);
  const fragment = shader(gl, gl.FRAGMENT_SHADER, FRAGMENT);
  const program = gl.createProgram();
  if (!program) throw new WebGlPainterCapabilityError('program allocation failed');
  gl.attachShader(program, vertex);
  gl.attachShader(program, fragment);
  gl.linkProgram(program);
  gl.deleteShader(vertex);
  gl.deleteShader(fragment);
  if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
    throw new WebGlPainterCapabilityError(gl.getProgramInfoLog(program) || 'program link failed');
  }
  const vao = gl.createVertexArray();
  const texture = gl.createTexture();
  if (!vao || !texture) throw new WebGlPainterCapabilityError('texture or vertex-array allocation failed');
  gl.bindVertexArray(vao);
  gl.activeTexture(gl.TEXTURE0);
  gl.bindTexture(gl.TEXTURE_2D, texture);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
  gl.useProgram(program);
  gl.uniform1i(gl.getUniformLocation(program, 'u_frame'), 0);
  gl.disable(gl.BLEND);
  gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, 0);
  gl.pixelStorei(gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL, 1);
  // Match importExternalTexture({ colorSpace: 'srgb' }) independently of
  // the output canvas's display space. Engine imports carry sRGB tags.
  gl.unpackColorSpace = 'srgb';
  gl.pixelStorei(gl.UNPACK_COLORSPACE_CONVERSION_WEBGL, gl.BROWSER_DEFAULT_WEBGL);

  let configured = '';
  let disposed = false;
  let lostReason: string | null = null;
  let resolveLost!: (reason: string) => void;
  const lost = new Promise<string>((resolve) => { resolveLost = resolve; });
  const fences = new Set<Fence>();
  const settleFence = (fence: Fence, error?: Error): void => {
    if (!fences.delete(fence)) return;
    if (fence.timer !== null) clearTimeout(fence.timer);
    if (!gl.isContextLost()) gl.deleteSync(fence.sync);
    if (error) fence.reject(error);
    else fence.resolve();
  };
  const markLost = (reason: string): void => {
    if (lostReason !== null) return;
    lostReason = reason;
    for (const fence of [...fences]) settleFence(fence);
    if (!disposed) resolveLost(reason);
  };
  const checkContext = (): void => {
    if (lostReason !== null || gl.isContextLost()) {
      markLost(lostReason || 'WebGL context lost');
      throw new DeviceLostError(lostReason!);
    }
  };
  const onContextLost = (event: Event): void => {
    event.preventDefault();
    markLost('WebGL context lost');
  };
  canvas.addEventListener('webglcontextlost', onContextLost);
  const checkError = (operation: string): void => {
    const error = gl.getError();
    if (error === gl.CONTEXT_LOST_WEBGL || gl.isContextLost()) {
      markLost(`WebGL context lost during ${operation}`);
      throw new DeviceLostError(lostReason!);
    }
    if (error !== gl.NO_ERROR) {
      // During a GPU-process restart the imported VideoFrame's mailbox
      // can become invalid before this context's loss event is dispatched.
      // No JS exception is thrown by texImage2D in that interval. Retire
      // this painter through the same loss path, never reuse that mailbox.
      if (error === gl.INVALID_VALUE || error === gl.INVALID_OPERATION) {
        markLost(`${operation} lost its graphics resource (GL 0x${error.toString(16)})`);
        throw new DeviceLostError(lostReason!);
      }
      throw new WebGlPainterCapabilityError(`${operation} failed (GL 0x${error.toString(16)})`);
    }
  };

  const painter: FramePainter = {
    lost,
    configure(format: SurfaceFormat, colorSpace: ViewerColorSpace, width: number, height: number): void {
      checkContext();
      if (disposed) throw new WebGlPainterCapabilityError('painter is disposed');
      if (format !== 'rgba16Float' && format !== 'bgra8Unorm') {
        throw new WebGlPainterCapabilityError(`unsupported source format ${format}`);
      }
      const displaySpace = colorSpace === 'displayP3' ? 'display-p3' : 'srgb';
      const key = `${format}:${displaySpace}:${width}x${height}`;
      if (key === configured) return;
      if (canvas.width !== width) canvas.width = width;
      if (canvas.height !== height) canvas.height = height;
      gl.drawingBufferColorSpace = displaySpace;
      if (gl.drawingBufferColorSpace !== displaySpace) {
        throw new WebGlPainterCapabilityError(`drawing-buffer color space ${displaySpace} was rejected`);
      }
      gl.drawingBufferStorage!(gl.RGBA16F, width, height);
      checkError('RGBA16F drawingBufferStorage');
      if (gl.drawingBufferFormat !== gl.RGBA16F) {
        throw new WebGlPainterCapabilityError('drawing buffer is not RGBA16F');
      }
      gl.viewport(0, 0, width, height);
      configured = key;
    },
    draw(frame: VideoFrame): void {
      checkContext();
      if (disposed) throw new WebGlPainterCapabilityError('painter is disposed');
      if (!configured) throw new WebGlPainterCapabilityError('painter is not configured');
      // Native imports deliberately carry BT.709/sRGB tags even when their
      // encoded values target a P3 canvas (bridge.ts::importColorSpace).
      // Chromium's WebGL VideoFrame upload ignores color-space conversion;
      // accepting a differently tagged source would silently change pixels.
      // SRGB_HDR (the F16 bridge tag) has no WebCodecs transfer enum and
      // appears as null; its code values still use the same sRGB curve.
      if (frame.colorSpace.primaries !== 'bt709' ||
          (frame.colorSpace.transfer !== null && frame.colorSpace.transfer !== 'iec61966-2-1')) {
        throw new WebGlPainterCapabilityError(`the imported frame must carry BT.709/sRGB tags (${JSON.stringify(frame.colorSpace.toJSON())})`);
      }
      gl.activeTexture(gl.TEXTURE0);
      gl.bindTexture(gl.TEXTURE_2D, texture);
      // VideoFrame is a WebGL TexImageSource. HALF_FLOAT must be accepted for
      // F16 input; an unsupported upload fails closed instead of using RGBA8.
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA16F, gl.RGBA, gl.HALF_FLOAT, frame);
      checkError('VideoFrame RGBA16F upload');
      gl.bindVertexArray(vao);
      gl.useProgram(program);
      gl.clearColor(0, 0, 0, 0);
      gl.clear(gl.COLOR_BUFFER_BIT);
      gl.drawArrays(gl.TRIANGLES, 0, 3);
      checkError('F16 presentation draw');
    },
    workDone(): Promise<void> {
      if (lostReason !== null || gl.isContextLost()) {
        markLost(lostReason || 'WebGL context lost');
        return Promise.resolve();
      }
      const sync = gl.fenceSync(gl.SYNC_GPU_COMMANDS_COMPLETE, 0);
      if (!sync) {
        try {
          checkError('GPU completion fence');
          return Promise.reject(new WebGlPainterCapabilityError('GPU completion fence allocation failed'));
        } catch (error) {
          // `workDone` is a promise API. Its caller can then retire the frame
          // through either settlement path, including during context loss.
          return Promise.reject(error);
        }
      }
      gl.flush();
      return new Promise<void>((resolve, reject) => {
        const fence: Fence = { sync, timer: null, resolve, reject };
        fences.add(fence);
        const poll = (): void => {
          fence.timer = null;
          if (gl.isContextLost()) {
            markLost('WebGL context lost while awaiting completion');
            return;
          }
          const status = gl.clientWaitSync(sync, 0, 0);
          if (status === gl.ALREADY_SIGNALED || status === gl.CONDITION_SATISFIED) {
            settleFence(fence);
          } else if (status === gl.WAIT_FAILED) {
            const error = gl.getError();
            if (error === gl.CONTEXT_LOST_WEBGL || gl.isContextLost()) {
              markLost('WebGL context lost while awaiting completion');
            } else {
              settleFence(fence, new WebGlPainterCapabilityError(`completion fence wait failed (GL 0x${error.toString(16)})`));
            }
          } else {
            fence.timer = setTimeout(poll, 0);
          }
        };
        fence.timer = setTimeout(poll, 0);
      });
    },
    dispose(): void {
      if (disposed) return;
      disposed = true;
      canvas.removeEventListener('webglcontextlost', onContextLost);
      if (!gl.isContextLost()) {
        // GL defers deletion until submitted work finishes. Any outstanding
        // fences continue polling so a caller cannot retire IOSurfaces early.
        gl.deleteTexture(texture);
        gl.deleteVertexArray(vao);
        gl.deleteProgram(program);
      } else {
        markLost('WebGL context lost');
      }
    }
  };
  return painter;
}
