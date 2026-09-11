import { describe, expect, it } from 'vitest';
import { displayResolution } from './display-resolution';

describe('export display resolution', () => {
  it('renders enlarged Retina output in physical pixels', () => {
    expect(displayResolution(320, 180, 960, 540, 2, 8192)).toEqual({ width: 1920, height: 1080 });
  });
  it('tracks smaller displays and fractional zoom without changing scene coordinates', () => {
    expect(displayResolution(1920, 1080, 640, 360, 1.5, 8192)).toEqual({ width: 960, height: 540 });
  });
  it('caps large displays while preserving the composition aspect ratio', () => {
    expect(displayResolution(320, 180, 10000, 5625, 3, 4096)).toEqual({ width: 4096, height: 2304 });
  });
});
