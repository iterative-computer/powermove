import { createWebGlPainter } from './webgl-painter';

// Electron 44's WebGPU external-texture cache dereferences a null cached
// shared image after graphics-context loss. Present the native surface through
// WebGL2 instead, preserving F16, premultiplied alpha and the display space.
export const createFramePainter = createWebGlPainter;
