import { createOpaqueReadback, type OpaqueReadbackSession } from './opaque-readback';

/** Copy the already displayed image, scaled and top-down, without another
 * composition render or a synchronous canvas/CPU read. One bounded capture. */
export function createPresentedReadback(gl: WebGL2RenderingContext | null, canvas: HTMLCanvasElement,
  width: number, height: number, valid: () => boolean): OpaqueReadbackSession | null {
  if (!gl || !valid() || gl.isContextLost() || !Number.isInteger(width) || !Number.isInteger(height)
      || width < 1 || height < 1 || width > 320 || height > 320) return null;
  const previousTexture = gl.getParameter(gl.TEXTURE_BINDING_2D);
  const previousDraw = gl.getParameter(gl.DRAW_FRAMEBUFFER_BINDING);
  let texture: WebGLTexture | null = null, framebuffer: WebGLFramebuffer | null = null;
  const release = () => { if (framebuffer) gl.deleteFramebuffer(framebuffer); if (texture) gl.deleteTexture(texture); framebuffer = texture = null; };
  try {
    texture = gl.createTexture(); framebuffer = gl.createFramebuffer();
    if (!texture || !framebuffer) { release(); return null; }
    gl.bindTexture(gl.TEXTURE_2D, texture);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, width, height, 0, gl.RGBA, gl.UNSIGNED_BYTE, null);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
    gl.bindFramebuffer(gl.DRAW_FRAMEBUFFER, framebuffer);
    gl.framebufferTexture2D(gl.DRAW_FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, texture, 0);
    if (gl.checkFramebufferStatus(gl.DRAW_FRAMEBUFFER) !== gl.FRAMEBUFFER_COMPLETE) { release(); return null; }
  } catch { release(); return null; }
  finally { gl.bindTexture(gl.TEXTURE_2D, previousTexture); gl.bindFramebuffer(gl.DRAW_FRAMEBUFFER, previousDraw); }
  let readback: OpaqueReadbackSession | null;
  try {
    readback = createOpaqueReadback(gl, width, height, (_time, _options, read) => {
      const oldRead = gl.getParameter(gl.READ_FRAMEBUFFER_BINDING), oldDraw = gl.getParameter(gl.DRAW_FRAMEBUFFER_BINDING);
      try {
        gl.bindFramebuffer(gl.READ_FRAMEBUFFER, null);
        gl.bindFramebuffer(gl.DRAW_FRAMEBUFFER, framebuffer);
        // Flip during the GPU copy: ImageData and JPEG both expect top-down rows.
        gl.blitFramebuffer(0, 0, canvas.width, canvas.height, 0, height, width, 0, gl.COLOR_BUFFER_BIT, gl.LINEAR);
        gl.bindFramebuffer(gl.READ_FRAMEBUFFER, framebuffer);
        read();
      } finally { gl.bindFramebuffer(gl.READ_FRAMEBUFFER, oldRead); gl.bindFramebuffer(gl.DRAW_FRAMEBUFFER, oldDraw); }
    }, valid);
  } catch { release(); return null; }
  if (!readback) { release(); return null; }
  const session = readback;
  return { begin: (time, options) => session.begin(time, options), dispose() { session.dispose(); release(); } };
}
