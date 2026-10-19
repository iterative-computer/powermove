import type { PowermoveAPI } from 'powermove';

// After each frame, a 48 x 24 red mark 8 px from the top-left of the frame
// in today's compositor canvas (render.gl.context), then a read of that
// canvas through drawImage(context.canvas): the mark and the frame's center.
export default function activate(api: PowermoveAPI): void {
  const w = window as any;
  w.__watermark = { frames: 0, times: [] as number[], reads: [] as unknown[] };
  const probe = document.createElement('canvas');
  probe.width = 1; probe.height = 1;
  const read = (source: HTMLCanvasElement, x: number, y: number) => {
    const c2 = probe.getContext('2d', { willReadFrequently: true })!;
    c2.clearRect(0, 0, 1, 1);
    c2.drawImage(source, x, y, 1, 1, 0, 0, 1, 1);
    return Array.from(c2.getImageData(0, 0, 1, 1).data);
  };
  api.events.on('frame:rendered', (frame: any) => {
    const gl = api.render.gl.context;
    if (!gl) return;
    const height = gl.drawingBufferHeight;
    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
    gl.viewport(0, 0, gl.drawingBufferWidth, height);
    gl.enable(gl.SCISSOR_TEST);
    gl.scissor(8, height - 8 - 24, 48, 24);
    gl.clearColor(232 / 255, 32 / 255, 42 / 255, 1);
    gl.clear(gl.COLOR_BUFFER_BIT);
    gl.disable(gl.SCISSOR_TEST);
    w.__watermark.frames++;
    w.__watermark.times.push(Number(frame?.time ?? NaN));
    const canvas = gl.canvas as HTMLCanvasElement;
    if (w.__watermark.reads.length < 8 && canvas.width > 64) {
      w.__watermark.reads.push({ size: [canvas.width, canvas.height], mark: read(canvas, 20, 16), center: read(canvas, Math.floor(canvas.width / 2), Math.floor(canvas.height / 2)) });
    }
  });
  api.transport.invalidate();
}
