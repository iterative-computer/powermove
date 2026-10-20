export interface OpaqueReadbackTicket {
  readInto(target: Uint8Array, cancelled: () => boolean): Promise<boolean>;
  dispose(): void;
}

export interface OpaqueReadbackSession {
  begin(time: number, options?: Record<string, unknown>): OpaqueReadbackTicket;
  dispose(): void;
}

/** One GPU buffer, one outstanding frame, and no owned CPU pixel storage.
 * The caller supplies the completed frame's storage after its preceding write
 * has acknowledged it, so capture can overlap delivery without another frame. */
export function createOpaqueReadback(
  gl: WebGL2RenderingContext | null,
  width: number,
  height: number,
  capture: (time: number, options: Record<string, unknown>, read: () => void) => void,
  valid: () => boolean,
): OpaqueReadbackSession | null {
  const bytes = width * height * 4;
  if (!gl || !Number.isInteger(width) || !Number.isInteger(height) || width <= 0 || height <= 0
      || !Number.isSafeInteger(bytes) || !gl.fenceSync || !gl.clientWaitSync || !gl.getBufferSubData
      || !valid() || gl.isContextLost()) return null;
  let buffer: WebGLBuffer | null = null;
  const previous = gl.getParameter(gl.PIXEL_PACK_BUFFER_BINDING);
  try {
    buffer = gl.createBuffer();
    if (!buffer) return null;
    gl.bindBuffer(gl.PIXEL_PACK_BUFFER, buffer);
    gl.bufferData(gl.PIXEL_PACK_BUFFER, bytes, gl.STREAM_READ);
    // WebGL allocation failures set an error instead of throwing.
    if (gl.getBufferParameter(gl.PIXEL_PACK_BUFFER, gl.BUFFER_SIZE) !== bytes || gl.isContextLost()) {
      gl.deleteBuffer(buffer); buffer = null;
    }
  } catch {
    if (buffer) gl.deleteBuffer(buffer);
    buffer = null;
  } finally { gl.bindBuffer(gl.PIXEL_PACK_BUFFER, previous); }
  if (!buffer) return null;

  let disposed = false, pending: OpaqueReadbackTicket | null = null;
  const available = () => !disposed && valid() && !gl.isContextLost();
  const failure = () => new Error('The GPU could not capture the export frame');
  return {
    begin(time, options = {}) {
      if (!available()) throw failure();
      if (pending) throw new Error('A GPU frame capture is already pending');
      let fence: WebGLSync | null = null, reading = false, finished = false;
      let timer: ReturnType<typeof setTimeout> | undefined;
      let wake: (() => void) | undefined;
      const finish = () => {
        if (finished) return;
        finished = true;
        clearTimeout(timer);
        if (fence) gl.deleteSync(fence);
        fence = null;
        if (pending === ticket) pending = null;
      };
      const ticket: OpaqueReadbackTicket = {
        async readInto(target, cancelled) {
          if (finished || disposed) { finish(); return false; }
          if (reading) throw new Error('A GPU frame capture is already being read');
          reading = true;
          try {
            if (cancelled()) return false;
            if (!(target instanceof Uint8Array) || target.length !== bytes) throw new Error('The GPU capture target has the wrong size or type');
            if (!available()) throw failure();
            const started = performance.now();
            const ready = await new Promise<boolean>((resolve, reject) => {
              const settle = (result: boolean, error?: Error) => {
                clearTimeout(timer); wake = undefined;
                if (error) reject(error); else resolve(result);
              };
              wake = () => settle(false);
              const poll = () => {
                try {
                  if (finished || disposed || cancelled()) { settle(false); return; }
                  if (!available()) throw failure();
                  if (performance.now() - started >= 10000) throw new Error('The GPU frame capture timed out');
                  const status = gl.clientWaitSync(fence!, 0, 0);
                  if (status === gl.ALREADY_SIGNALED || status === gl.CONDITION_SATISFIED) settle(true);
                  else if (status === gl.WAIT_FAILED) throw failure();
                  else timer = setTimeout(poll, 0);
                } catch (error) { settle(false, error instanceof Error ? error : failure()); }
              };
              // WebGL fences cannot signal until control returns to the event
              // loop. A microtask-only wait would never make progress.
              timer = setTimeout(poll, 0);
            });
            if (!ready || finished || disposed || cancelled()) return false;
            if (!available() || target.length !== bytes) throw failure();
            const old = gl.getParameter(gl.PIXEL_PACK_BUFFER_BINDING);
            try {
              gl.bindBuffer(gl.PIXEL_PACK_BUFFER, buffer);
              gl.getBufferSubData(gl.PIXEL_PACK_BUFFER, 0, target);
            } finally { gl.bindBuffer(gl.PIXEL_PACK_BUFFER, old); }
            if (!available()) throw failure();
            return true;
          } finally { wake = undefined; finish(); }
        },
        dispose() { wake?.(); finish(); },
      };
      // Reserve the ticket before rendering, including reentrant captures.
      pending = ticket;
      try {
        let queued = false;
        capture(time, options, () => {
          if (finished || !available()) throw failure();
          if (queued) throw new Error('A GPU frame capture was queued twice');
          queued = true;
          const old = gl.getParameter(gl.PIXEL_PACK_BUFFER_BINDING);
          try {
            gl.bindBuffer(gl.PIXEL_PACK_BUFFER, buffer);
            gl.readPixels(0, 0, width, height, gl.RGBA, gl.UNSIGNED_BYTE, 0);
            fence = gl.fenceSync(gl.SYNC_GPU_COMMANDS_COMPLETE, 0);
            if (!fence) throw failure();
            gl.flush();
          } finally { gl.bindBuffer(gl.PIXEL_PACK_BUFFER, old); }
        });
        if (!queued || !available()) throw failure();
        return ticket;
      } catch (error) { ticket.dispose(); throw error; }
    },
    dispose() {
      if (disposed) return;
      disposed = true;
      pending?.dispose();
      gl.deleteBuffer(buffer); buffer = null;
    },
  };
}
