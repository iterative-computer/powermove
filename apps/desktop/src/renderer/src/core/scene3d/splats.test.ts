import { describe, expect, it } from 'vitest';
import { parsePly, parseSplat } from './splats';

/** A 3DGS PLY: x y z, f_dc_0..2, opacity, scale_0..2, rot_0..3 (all float). */
function gaussianPly(splats: { p: number[]; dc: number[]; opacity: number; scale: number[]; rot: number[] }[]): ArrayBuffer {
  const names = ['x', 'y', 'z', 'f_dc_0', 'f_dc_1', 'f_dc_2', 'opacity', 'scale_0', 'scale_1', 'scale_2', 'rot_0', 'rot_1', 'rot_2', 'rot_3'];
  const header = `ply\nformat binary_little_endian 1.0\nelement vertex ${splats.length}\n${names.map(n => `property float ${n}`).join('\n')}\nend_header\n`;
  const head = new TextEncoder().encode(header);
  const out = new Uint8Array(head.length + splats.length * names.length * 4);
  out.set(head);
  const v = new DataView(out.buffer, head.length);
  splats.forEach((s, i) => [...s.p, ...s.dc, s.opacity, ...s.scale, ...s.rot].forEach((x, k) => v.setFloat32((i * names.length + k) * 4, x, true)));
  return out.buffer;
}

describe('Gaussian splat files', () => {
  it('reads 3DGS PLY: color from the DC coefficients, sigmoid opacity, covariance from scale and rotation', () => {
    const d = parsePly(gaussianPly([
      { p: [1, 2, 3], dc: [0, 0, 0], opacity: 0, scale: [Math.log(2), Math.log(2), Math.log(2)], rot: [1, 0, 0, 0] },
      // Stretched along x, then turned 90° about z: the long axis lies along y.
      { p: [0, 0, 0], dc: [1.7724538, -1.7724538, 0], opacity: 100, scale: [Math.log(3), 0, 0], rot: [Math.SQRT1_2, 0, 0, Math.SQRT1_2] },
    ]));
    expect(d.count).toBe(2);
    expect([...d.centers.subarray(0, 3)]).toEqual([1, 2, 3]);
    // DC 0 is mid grey; opacity 0 → sigmoid 0.5.
    expect([...d.colors.subarray(0, 4)]).toEqual([128, 128, 128, 128]);
    expect([...d.colors.subarray(4, 8)]).toEqual([255, 0, 128, 255]);
    // Isotropic scale 2: Σ = 4·I.
    expect([...d.covariances.subarray(0, 6)].map(v => +v.toFixed(4))).toEqual([4, 0, 0, 4, 0, 4]);
    expect(d.covariances[9]).toBeCloseTo(9); // yy
    expect(d.covariances[6]).toBeCloseTo(1); // xx
  });

  it('reads the 32-byte .splat layout', () => {
    const buf = new ArrayBuffer(32), f = new Float32Array(buf), u = new Uint8Array(buf);
    f.set([0.5, -1, 2, 0.1, 0.1, 0.1]);
    u.set([10, 20, 30, 200, 255, 128, 128, 128], 24);
    const d = parseSplat(buf);
    expect([...d.centers]).toEqual([0.5, -1, 2]);
    expect([...d.colors]).toEqual([10, 20, 30, 200]);
    expect(d.covariances[0]).toBeCloseTo(0.01);
    expect(() => parseSplat(new ArrayBuffer(31))).toThrow('32 bytes');
  });

  it('turns a plain colored point cloud into round splats', () => {
    const header = 'ply\nformat binary_little_endian 1.0\nelement vertex 2\nproperty float x\nproperty float y\nproperty float z\nproperty uchar red\nproperty uchar green\nproperty uchar blue\nend_header\n';
    const head = new TextEncoder().encode(header), out = new Uint8Array(head.length + 2 * 15), v = new DataView(out.buffer, head.length);
    out.set(head);
    [[0, 0, 0, 255, 0, 0], [1, 1, 1, 0, 0, 255]].forEach((p, i) => { for (let k = 0; k < 3; k++) v.setFloat32(i * 15 + k * 4, p[k]!, true); for (let k = 0; k < 3; k++) v.setUint8(i * 15 + 12 + k, p[3 + k]!); });
    const d = parsePly(out.buffer);
    expect([...d.colors]).toEqual([255, 0, 0, 255, 0, 0, 255, 255]);
    expect(d.covariances[0]).toBeGreaterThan(0);
    expect(d.covariances[1]).toBe(0);
  });
});
