let gl: WebGL2RenderingContext | null | undefined;

/**
 * A canvas's pixels as the compositor's texture holds them: uploaded to a
 * WebGL2 texture with `UNPACK_PREMULTIPLY_ALPHA_WEBGL` and read back, rows
 * top-down. Without WebGL2, the 2D context's straight-alpha pixels.
 */
export function compositorPixels(source: CanvasImageSource & { width: number; height: number }): { width: number; height: number; format: 'rgba8Premultiplied' | 'rgba8Straight'; data: Uint8Array } | null {
  const width = Math.round(source.width), height = Math.round(source.height);
  if (!(width >= 1 && height >= 1)) return null;
  if (gl === undefined) {
    try {
      gl = typeof OffscreenCanvas !== 'undefined' ? new OffscreenCanvas(1, 1).getContext('webgl2', { premultipliedAlpha: true, antialias: false }) : null;
    } catch {
      gl = null;
    }
  }
  if (gl && !gl.isContextLost()) {
    const texture = gl.createTexture();
    const framebuffer = gl.createFramebuffer();
    try {
      gl.bindTexture(gl.TEXTURE_2D, texture);
      gl.pixelStorei(gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL, true);
      gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, false);
      gl.pixelStorei(gl.UNPACK_COLORSPACE_CONVERSION_WEBGL, gl.NONE);
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, source as TexImageSource);
      gl.bindFramebuffer(gl.FRAMEBUFFER, framebuffer);
      gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, texture, 0);
      if (gl.checkFramebufferStatus(gl.FRAMEBUFFER) === gl.FRAMEBUFFER_COMPLETE) {
        const data = new Uint8Array(width * height * 4);
        gl.readPixels(0, 0, width, height, gl.RGBA, gl.UNSIGNED_BYTE, data);
        return { width, height, format: 'rgba8Premultiplied', data };
      }
    } catch {
      // Fall through to the 2D context.
    } finally {
      gl.bindFramebuffer(gl.FRAMEBUFFER, null);
      gl.deleteFramebuffer(framebuffer);
      gl.deleteTexture(texture);
    }
  }
  const canvas = typeof OffscreenCanvas !== 'undefined' ? new OffscreenCanvas(width, height) : Object.assign(document.createElement('canvas'), { width, height });
  const context = (canvas as HTMLCanvasElement).getContext('2d') as CanvasRenderingContext2D | null;
  if (!context) return null;
  context.drawImage(source, 0, 0);
  const pixels = context.getImageData(0, 0, width, height).data;
  return { width, height, format: 'rgba8Straight', data: new Uint8Array(pixels.buffer, pixels.byteOffset, pixels.byteLength) };
}
