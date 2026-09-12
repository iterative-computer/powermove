/** Keep composition coordinates independent from the physical drawing buffer. */
export function displayResolution(width: number, height: number, cssWidth: number, cssHeight: number, pixelRatio: number, limit: number) {
  const density = Number.isFinite(pixelRatio) && pixelRatio > 0 ? pixelRatio : 1;
  const scale = Math.min(Math.max(cssWidth / width, cssHeight / height, 0) * density || 1,
    limit / width, limit / height);
  return { width: Math.max(1, Math.round(width * scale)), height: Math.max(1, Math.round(height * scale)) };
}

export function observeDisplayResolution(canvas: HTMLCanvasElement, width: number, height: number,
  limit: number, resize: (width: number, height: number) => void, invalidate: () => void) {
  let dead = false;
  const update = () => {
    if (dead) return false;
    const rect = canvas.getBoundingClientRect();
    // Hidden/detached players retain their last useful buffer until shown.
    if (!rect.width || !rect.height) return false;
    const next = displayResolution(width, height, rect.width, rect.height, window.devicePixelRatio, limit);
    if (canvas.width === next.width && canvas.height === next.height) return false;
    const style = getComputedStyle(canvas), cssWidth = style.width, cssHeight = style.height;
    resize(next.width, next.height);
    // An unstyled canvas derives its CSS size from its buffer. Break that
    // feedback loop without replacing responsive CSS on sized canvases.
    const after = canvas.getBoundingClientRect();
    if (Math.abs(after.width - rect.width) > .5) canvas.style.width = cssWidth;
    if (Math.abs(canvas.getBoundingClientRect().height - rect.height) > .5) canvas.style.height = cssHeight;
    return true;
  };
  const refresh = () => { if (update()) invalidate(); };
  const observer = new ResizeObserver(refresh);
  observer.observe(canvas);
  let query: MediaQueryList;
  const watchDensity = () => {
    query?.removeEventListener('change', densityChanged);
    query = matchMedia(`(resolution: ${window.devicePixelRatio}dppx)`);
    query.addEventListener('change', densityChanged);
  };
  const densityChanged = () => { watchDensity(); refresh(); };
  watchDensity();
  window.addEventListener('resize', refresh);
  return { update, destroy() { dead = true; observer.disconnect(); query.removeEventListener('change', densityChanged); window.removeEventListener('resize', refresh); } };
}
