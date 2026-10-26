/** CSS inset(... round ...) uses circular corners, unlike our panel superellipse. */
export function panelClipPath(width: number, top: number, bottom: number, radius: number, smoothing: number) {
  const r = Math.min(radius, width / 2, Math.max(0, bottom - top) / 2);
  const power = 2 / Math.pow(2, smoothing);
  const points: string[] = [];
  const corners: ReadonlyArray<readonly [number, number, number]> = [
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

/* A panel clipped by a column carries this attribute, so whichever column
   holds it next clears the clip. A panel can move between columns (a
   workspace switch, a drag) between two frames; a clip kept from its old
   column would cut its bottom off and read as a gap in the new one. */
const CLIPPED = 'scrollClipped';

function unclip(panel: HTMLElement): void {
  if (panel.dataset[CLIPPED] === undefined) return;
  panel.style.clipPath = '';
  panel.style.borderRadius = '';
  delete panel.dataset[CLIPPED];
}

/** Round the visible part of a panel, including while it crosses a dock edge. */
export function roundedScroll(node: HTMLElement) {
  if (node.id === 'dock-center') return;
  let frame = 0;
  const clipped = new Set<HTMLElement>();
  /* Tell the column whether content is hidden above or below, so CSS can fade
     that edge. Without it a full column looks identical to a scrollable one. */
  const markScrollEdges = () => {
    const remaining = node.scrollHeight - node.clientHeight;
    // Sub-pixel dock heights leave a fractional remainder at either end.
    const scrollable = remaining > 1;
    node.dataset.overflowTop = scrollable && node.scrollTop > 1 ? '1' : '0';
    node.dataset.overflowBottom = scrollable && node.scrollTop < remaining - 1 ? '1' : '0';
  };
  const update = () => {
    frame = 0;
    markScrollEdges();
    const bounds = node.getBoundingClientRect();
    const chrome = getComputedStyle(node);
    const radius = parseFloat(chrome.borderTopLeftRadius);
    const smoothing = Number(chrome.getPropertyValue('--ui-corner-smoothing').match(/superellipse\(([-\d.]+)\)/)?.[1] ?? 1);
    const panels = node.querySelectorAll<HTMLElement>(':scope > .panel-slot > .panel');
    // Panels that left this column take their clip off with them.
    for (const panel of clipped) if (!node.contains(panel)) { unclip(panel); clipped.delete(panel); }
    for (const panel of panels) {
      const rect = panel.getBoundingClientRect();
      const top = Math.min(rect.height, Math.max(0, bounds.top - rect.top));
      const bottom = Math.min(rect.height - top, Math.max(0, rect.bottom - bounds.bottom));
      if (!top && !bottom) {
        unclip(panel);
        clipped.delete(panel);
        continue;
      }
      // Avoid intersecting the panel's original corners with the viewport's
      // corners: that intersection makes a lens out of a thin visible slice.
      panel.style.borderRadius = '0px';
      panel.style.clipPath = panelClipPath(rect.width, top, rect.height - bottom, radius, smoothing);
      panel.dataset[CLIPPED] = '';
      clipped.add(panel);
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
    for (const panel of clipped) unclip(panel);
    delete node.dataset.overflowTop;
    delete node.dataset.overflowBottom;
  } };
}
