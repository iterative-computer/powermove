/** CSS inset(... round ...) uses circular corners, unlike our panel superellipse. */
export function panelClipPath(width: number, top: number, bottom: number, radius: number, smoothing: number) {
  const r = Math.min(radius, width / 2, Math.max(0, bottom - top) / 2);
  const power = 2 / Math.pow(2, smoothing);
  const points: string[] = [];
  const corners = [
    [width - r, top + r, -Math.PI / 2],
    [width - r, bottom - r, 0],
    [r, bottom - r, Math.PI / 2],
    [r, top + r, Math.PI]
  ];
  for (const [cx, cy, start] of corners) {
    for (let step = 0; step <= 24; step++) {
      const angle = start + step / 24 * Math.PI / 2;
      const x = Math.cos(angle), y = Math.sin(angle);
      points.push(`${points.length ? 'L' : 'M'}${(cx + r * Math.sign(x) * Math.abs(x) ** power).toFixed(3)} ${(cy + r * Math.sign(y) * Math.abs(y) ** power).toFixed(3)}`);
    }
  }
  return `path('${points.join(' ')} Z')`;
}

/** Round the visible part of a panel, including while it crosses a dock edge. */
export function roundedScroll(node: HTMLElement) {
  if (node.id === 'dock-center') return;
  const originals = new Map<HTMLElement, { clipPath: string; borderRadius: string }>();
  let frame = 0;
  const restore = (panel: HTMLElement) => {
    const original = originals.get(panel);
    if (!original) return;
    panel.style.clipPath = original.clipPath;
    panel.style.borderRadius = original.borderRadius;
    originals.delete(panel);
  };
  const update = () => {
    frame = 0;
    const bounds = node.getBoundingClientRect();
    const chrome = getComputedStyle(node);
    const radius = parseFloat(chrome.borderTopLeftRadius);
    const smoothing = Number(chrome.getPropertyValue('--ui-corner-smoothing').match(/superellipse\(([-\d.]+)\)/)?.[1] ?? 1);
    const panels = node.querySelectorAll<HTMLElement>(':scope > .panel-slot > .panel');
    for (const panel of originals.keys()) if (!node.contains(panel)) restore(panel);
    for (const panel of panels) {
      const rect = panel.getBoundingClientRect();
      const top = Math.min(rect.height, Math.max(0, bounds.top - rect.top));
      const bottom = Math.min(rect.height - top, Math.max(0, rect.bottom - bounds.bottom));
      if (!top && !bottom) {
        restore(panel);
        continue;
      }
      if (!originals.has(panel)) originals.set(panel, {
        clipPath: panel.style.clipPath, borderRadius: panel.style.borderRadius
      });
      // Avoid intersecting the panel's original corners with the viewport's
      // corners: that intersection makes a lens out of a thin visible slice.
      panel.style.borderRadius = '0px';
      panel.style.clipPath = panelClipPath(rect.width, top, rect.height - bottom, radius, smoothing);
    }
  };
  const schedule = () => { if (!frame) frame = requestAnimationFrame(update); };
  const resize = new ResizeObserver(schedule);
  const observe = () => {
    resize.disconnect();
    resize.observe(node);
    node.querySelectorAll<HTMLElement>(':scope > .panel-slot > .panel').forEach(panel => resize.observe(panel));
    schedule();
  };
  const mutations = new MutationObserver(observe);
  mutations.observe(node, { childList: true, subtree: true });
  node.addEventListener('scroll', schedule, { passive: true });
  observe();
  return { destroy() {
    cancelAnimationFrame(frame);
    node.removeEventListener('scroll', schedule);
    resize.disconnect();
    mutations.disconnect();
    for (const panel of originals.keys()) restore(panel);
  } };
}
