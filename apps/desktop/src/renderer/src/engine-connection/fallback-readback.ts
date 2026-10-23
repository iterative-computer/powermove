/* Bounded WebGL2 pixel-pack ring. Submission stays synchronous; GPU fences
 * and CPU copies complete independently, so a frame does not stall the page
 * or prevent the following frame from being submitted. */
import { toHalfArray } from './half-float';

type Slot = { buffer: WebGLBuffer; bytes: number };
const CAPACITY = 3;
const DEADLINE_MS = 10_000;

export class FallbackReadback {
  private free: Slot[] = [];
  private active = 0;
  private disposed = false;
  private waiters: (() => void)[] = [];
  private readsHalf: boolean | null = null;

  constructor(private readonly gl: WebGL2RenderingContext) {}

  async ready(): Promise<void> {
    while (this.active >= CAPACITY && !this.disposed) await new Promise<void>(resolve => this.waiters.push(resolve));
    if (this.disposed) throw new Error('Fallback readback was disposed');
  }

  private half(): boolean {
    if (this.readsHalf !== null) return this.readsHalf;
    const gl = this.gl;
    return this.readsHalf = gl.getParameter(gl.IMPLEMENTATION_COLOR_READ_FORMAT) === gl.RGBA
      && gl.getParameter(gl.IMPLEMENTATION_COLOR_READ_TYPE) === gl.HALF_FLOAT;
  }

  /** Called after ready(), with the output framebuffer bound. */
  async read(width: number, height: number, colorDepth: 8 | 16 = 16): Promise<Uint16Array> {
    if (this.disposed || this.active >= CAPACITY) throw new Error('Fallback readback has no free slot');
    const gl = this.gl;
    const half = colorDepth === 16 && this.half();
    const count = width * height * 4;
    const bytes = count * (colorDepth === 8 ? 1 : half ? 2 : 4);
    const slot = this.free.pop() ?? { buffer: gl.createBuffer()!, bytes: 0 };
    let fence: WebGLSync | null = null;
    this.active++;
    try {
      if (!slot.buffer) throw new Error('Cannot allocate a fallback pixel buffer');
      gl.bindBuffer(gl.PIXEL_PACK_BUFFER, slot.buffer);
      if (slot.bytes !== bytes) {
        gl.bufferData(gl.PIXEL_PACK_BUFFER, bytes, gl.STREAM_READ);
        slot.bytes = bytes;
      }
      gl.readPixels(0, 0, width, height, gl.RGBA, colorDepth === 8 ? gl.UNSIGNED_BYTE : half ? gl.HALF_FLOAT : gl.FLOAT, 0);
      fence = gl.fenceSync(gl.SYNC_GPU_COMMANDS_COMPLETE, 0);
      gl.bindBuffer(gl.PIXEL_PACK_BUFFER, null);
      if (!fence || gl.getError() !== gl.NO_ERROR) throw new Error('Cannot submit fallback pixel readback');
      gl.flush();
      const deadline = performance.now() + DEADLINE_MS;
      for (;;) {
        if (this.disposed || gl.isContextLost()) throw new Error('Fallback readback context ended');
        const status = gl.clientWaitSync(fence, 0, 0);
        if (status === gl.ALREADY_SIGNALED || status === gl.CONDITION_SATISFIED) break;
        if (status === gl.WAIT_FAILED) throw new Error('Fallback readback fence failed');
        if (performance.now() >= deadline) throw new Error('Fallback readback exceeded its deadline');
        // WebGL cannot observe a newly signalled fence until the browser
        // has yielded to its event loop. Never busy-wait on the page.
        await new Promise(resolve => setTimeout(resolve, 1));
      }
      const pixels = colorDepth === 8 ? new Uint8Array(count) : half ? new Uint16Array(count) : new Float32Array(count);
      gl.bindBuffer(gl.PIXEL_PACK_BUFFER, slot.buffer);
      gl.getBufferSubData(gl.PIXEL_PACK_BUFFER, 0, pixels);
      gl.bindBuffer(gl.PIXEL_PACK_BUFFER, null);
      if (gl.getError() !== gl.NO_ERROR) throw new Error('Cannot copy fallback pixel readback');
      return colorDepth === 8 ? Uint16Array.from(pixels as Uint8Array) : half ? pixels as Uint16Array : toHalfArray(pixels as Float32Array);
    } finally {
      gl.bindBuffer(gl.PIXEL_PACK_BUFFER, null);
      if (fence) gl.deleteSync(fence);
      if (this.disposed || gl.isContextLost()) gl.deleteBuffer(slot.buffer);
      else this.free.push(slot);
      this.active--;
      this.waiters.shift()?.();
    }
  }

  dispose(): void {
    this.disposed = true;
    for (const slot of this.free) this.gl.deleteBuffer(slot.buffer);
    this.free = [];
    for (const wake of this.waiters.splice(0)) wake();
  }
}
