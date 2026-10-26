import { expect, it } from 'vitest';
import { gridSize, mipBytes, sampleLod } from './effect-sampling';
it('bounds wide kernels by projected radius and available mip dimensions', () => {
  expect(sampleLod(100, 3, 32, [1, 1], 512, 256)).toBe(4);
  expect(sampleLod(100, 3, 32, [.5, .5], 256, 128)).toBe(3);
  expect(sampleLod(100, 3, 32, [2, 1], 1024, 256)).toBe(5);
  expect(sampleLod(400, 3, 32, [8, 8], 2, 1)).toBe(1);
  expect(sampleLod(0, 3, 32, [1, 1], 512, 256)).toBe(0);
  expect(sampleLod(NaN, 3, 32, [1, 1], 512, 256)).toBe(0);
});
it('accounts every allocated mip level in the working precision, including thin textures', () => {
  expect(mipBytes(8, 8, 16, 3)).toBe((16 + 4 + 1) * 8);
  expect(mipBytes(8, 1, 32, 3)).toBe((4 + 2 + 1) * 16);
  expect(mipBytes(7, 5, 8, 2)).toBe((3 * 2 + 1) * 4);
  expect(mipBytes(8, 8, 16, 0)).toBe(0);
});
it('rounds independent grid dimensions and clamps malformed or oversized requests', () => {
  expect(gridSize(10.4, 7.6, 2048)).toEqual([10, 8]);
  expect(gridSize(Infinity, -3, 2048)).toEqual([1, 1]);
  expect(gridSize(4000, 40, 1024)).toEqual([1024, 40]);
});
