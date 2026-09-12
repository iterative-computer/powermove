// Batch only newly mounted siblings. Existing words never replay when a stream
// extends its last word, and inline spans retain native wrapping and selection.
const batches = new WeakMap<Element, number>();
function motion(node: HTMLElement, enabled: boolean, frames: Keyframe[], options: KeyframeAnimationOptions) {
  const media = window.matchMedia?.('(prefers-reduced-motion: reduce)');
  const animation = enabled && !media?.matches ? node.animate?.(frames, options) : undefined;
  const settle = () => { if (media?.matches) animation?.cancel(); };
  media?.addEventListener?.('change', settle);
  return { destroy() {
    animation?.cancel();
    media?.removeEventListener?.('change', settle);
  } };
}
export function revealText(node: HTMLElement, enabled = true): { destroy(): void } {
  const parent = node.parentElement;
  let index = 0;
  if (enabled && parent) {
    index = batches.get(parent) || 0;
    batches.set(parent, index + 1);
    if (!index) queueMicrotask(() => batches.delete(parent));
  }
  return motion(node, enabled, [{ opacity: 0 }, { opacity: 1 }], {
    duration: 200, delay: Math.min(index * 28, 280), easing: 'ease-out', fill: 'backwards'
  });
}
/** Mount-only: clearing the transient entering flag must not cut a send short. */
export function sendMessage(node: HTMLElement, enabled = true): { destroy(): void } {
  return motion(node, enabled, [
    { opacity: 0.3, transform: 'translateY(14px) scale(0.97)', transformOrigin: 'bottom right' },
    { opacity: 1, transform: 'translateY(-1px) scale(1.003)', transformOrigin: 'bottom right', offset: 0.75 },
    { opacity: 1, transform: 'none', transformOrigin: 'bottom right' }
  ], { duration: 360, easing: 'cubic-bezier(0.2, 0.75, 0.25, 1)' });
}
