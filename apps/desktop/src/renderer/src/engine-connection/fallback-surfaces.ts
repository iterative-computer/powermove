/* Reusable RGBA8/RGBA16F runner storage. Inputs are matched by exact pixels, not
 * object identity: callers may mutate or reassemble the same input buffer.
 * Leases protect a suspended multipass draw from synchronous snapshots. */
type Entry = { texture: WebGLTexture; framebuffer: WebGLFramebuffer | null; width: number; height: number; colorDepth: 8 | 16; pixels: Uint16Array | null; leases: number; used: number; deleted: boolean };
export type SurfaceLease = { texture: WebGLTexture; framebuffer: WebGLFramebuffer | null; release(): void };
const BUDGET = 128 * 1024 ** 2;
const LIMIT = 12;

export class FallbackSurfaces {
  private entries: Entry[] = [];
  private tick = 0;
  private disposed = false;
  constructor(private readonly gl: WebGL2RenderingContext) {}

  private same(a: Uint16Array, b: Uint16Array): boolean {
    if (a.length !== b.length) return false;
    // Pixel buffers are normally aligned. Preserve the view's byte offset
    // and still accept an unaligned uint16 view without copying it first.
    if (a.byteOffset % 4 === 0 && b.byteOffset % 4 === 0) {
      const n = Math.floor(a.length / 2);
      const aa = new Uint32Array(a.buffer, a.byteOffset, n), bb = new Uint32Array(b.buffer, b.byteOffset, n);
      for (let i = 0; i < n; i++) if (aa[i] !== bb[i]) return false;
      return a.length % 2 === 0 || a[a.length - 1] === b[b.length - 1];
    }
    for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) return false;
    return true;
  }

  input(width: number, height: number, pixels: Uint16Array, colorDepth: 8 | 16 = 16): SurfaceLease {
    const found = this.entries.find(e => !e.deleted && e.width === width && e.height === height && e.colorDepth === colorDepth && e.pixels && this.same(e.pixels, pixels));
    return this.lease(found ?? this.make(width, height, pixels, colorDepth));
  }

  target(width: number, height: number, colorDepth: 8 | 16 = 16): SurfaceLease {
    const found = this.entries.find(e => !e.deleted && !e.leases && e.width === width && e.height === height && e.colorDepth === colorDepth && e.framebuffer);
    return this.lease(found ?? this.make(width, height, null, colorDepth));
  }

  private make(width: number, height: number, pixels: Uint16Array | null, colorDepth: 8 | 16): Entry {
    if (this.disposed) throw new Error('Fallback surface cache was disposed');
    const gl = this.gl;
    const texture = gl.createTexture();
    if (!texture) throw new Error('Cannot allocate a fallback texture');
    const entry: Entry = { texture, framebuffer: null, width, height, colorDepth, pixels: pixels ? new Uint16Array(pixels) : null, leases: 0, used: ++this.tick, deleted: false };
    try {
      gl.activeTexture(gl.TEXTURE0);
      gl.bindTexture(gl.TEXTURE_2D, texture);
      gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
      const data = colorDepth === 8 && entry.pixels ? Uint8Array.from(entry.pixels, v => {
        if (v > 255) throw new Error('RGBA8 fallback code is out of range');
        return v;
      }) : entry.pixels;
      gl.texImage2D(gl.TEXTURE_2D, 0, colorDepth === 8 ? gl.RGBA8 : gl.RGBA16F, width, height, 0, gl.RGBA, colorDepth === 8 ? gl.UNSIGNED_BYTE : gl.HALF_FLOAT, data);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
      gl.bindTexture(gl.TEXTURE_2D, null);
      if (!pixels) {
        entry.framebuffer = gl.createFramebuffer();
        if (!entry.framebuffer) throw new Error('Cannot allocate a fallback framebuffer');
        gl.bindFramebuffer(gl.FRAMEBUFFER, entry.framebuffer);
        gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, texture, 0);
      }
      this.entries.push(entry);
      return entry;
    } catch (error) { this.remove(entry); throw error; }
  }

  private lease(entry: Entry): SurfaceLease {
    entry.leases++; entry.used = ++this.tick;
    let released = false;
    return { texture: entry.texture, framebuffer: entry.framebuffer, release: () => {
      if (released) return;
      released = true; entry.leases--;
      this.trim();
    } };
  }

  private trim(): void {
    // Include the CPU snapshot in the input's budget. Active leases can
    // exceed it only while a draw requires them; no oversized idle cache.
    const bytes = (e: Entry) => e.width * e.height * (e.colorDepth === 8 ? 4 : 8) + (e.pixels ? e.width * e.height * 8 : 0);
    let total = this.entries.reduce((n, e) => n + bytes(e), 0);
    while (total > BUDGET || this.entries.length > LIMIT) {
      const oldest = this.entries.filter(e => !e.leases).sort((a, b) => a.used - b.used)[0];
      if (!oldest) break;
      total -= bytes(oldest);
      this.remove(oldest);
      this.entries.splice(this.entries.indexOf(oldest), 1);
    }
  }

  private remove(entry: Entry): void {
    if (entry.deleted) return;
    entry.deleted = true;
    if (entry.framebuffer) this.gl.deleteFramebuffer(entry.framebuffer);
    this.gl.deleteTexture(entry.texture);
  }

  dispose(): void {
    this.disposed = true;
    for (const entry of this.entries) this.remove(entry);
    this.entries = [];
  }
}
