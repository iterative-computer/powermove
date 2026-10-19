import { describe, expect, it } from 'vitest';
import { FallbackSurfaces } from './fallback-surfaces';

function fixture() {
  let uploads = 0, deleted = 0, id = 0;
  const gl = {
    createTexture: () => ({ id: ++id }), createFramebuffer: () => ({ id: ++id }),
    activeTexture() {}, bindTexture() {}, pixelStorei() {}, texImage2D() { uploads++; },
    texParameteri() {}, bindFramebuffer() {}, framebufferTexture2D() {}, deleteFramebuffer() {},
    deleteTexture() { deleted++; }
  } as unknown as WebGL2RenderingContext;
  return { pool: new FallbackSurfaces(gl), uploads: () => uploads, deleted: () => deleted };
}

describe('fallback surface leases', () => {
  it('reuses exact input pixels across new buffers but notices in-place mutation and view offsets', () => {
    const f = fixture();
    const values = new Uint16Array(12); values.fill(0x3800);
    const input = values.subarray(1, 9); // intentionally not uint32 aligned
    const first = f.pool.input(2, 1, input); first.release();
    const same = f.pool.input(2, 1, new Uint16Array(input)); same.release();
    expect(same.texture).toBe(first.texture); expect(f.uploads()).toBe(1);
    input[3] = 0x3c00;
    const changed = f.pool.input(2, 1, input); changed.release();
    expect(changed.texture).not.toBe(first.texture); expect(f.uploads()).toBe(2);
    f.pool.dispose(); expect(f.deleted()).toBe(2);
  });

  it('keeps a suspended draw target leased while a synchronous snapshot gets its own target', () => {
    const f = fixture();
    const pending = f.pool.target(32, 18);
    const snapshot = f.pool.target(32, 18);
    expect(snapshot.texture).not.toBe(pending.texture);
    snapshot.release();
    const next = f.pool.target(32, 18);
    expect(next.texture).toBe(snapshot.texture);
    expect(f.uploads()).toBe(2);
    pending.release(); next.release(); f.pool.dispose();
  });

  it('does not retain an oversized target after release and tolerates disposal with outstanding leases', () => {
    const f = fixture();
    const huge = f.pool.target(8192, 8192);
    expect(f.deleted()).toBe(0);
    huge.release(); expect(f.deleted()).toBe(1);
    const active = f.pool.target(16, 16);
    f.pool.dispose(); expect(f.deleted()).toBe(2);
    active.release(); active.release(); expect(f.deleted()).toBe(2);
    expect(() => f.pool.target(16, 16)).toThrow('disposed');
  });
});
