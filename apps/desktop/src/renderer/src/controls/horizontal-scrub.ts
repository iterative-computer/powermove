/** Group a horizontal trackpad gesture into one edit, leaving vertical scrolling alone. */
import { bridge } from '../kernel/bridge';
export function horizontalScrub(node: HTMLElement, options: {
  begin: () => void; move: (delta: number, event: WheelEvent) => boolean | void;
  commit: () => void; cancel: () => void; enabled?: () => boolean;
}) {
  let travel = 0;
  let lastHaptic = -Infinity;
  let timer: ReturnType<typeof setTimeout> | undefined;
  const finish = () => { if (timer === undefined) return; clearTimeout(timer); timer = undefined; options.commit(); };
  const wheel = (event: WheelEvent) => {
    if (options.enabled?.() === false || event.ctrlKey || !event.deltaX || Math.abs(event.deltaX) <= Math.abs(event.deltaY)) return;
    event.preventDefault(); event.stopPropagation();
    if (timer === undefined) { travel = 0; options.begin(); }
    else clearTimeout(timer);
    const scale = event.deltaMode === 1 ? 16 : event.deltaMode === 2 ? node.clientWidth : 1;
    // Keep the trackpad's native acceleration and fractional deltas, at a
    // gentler gain than direct dragging. No extra inertia after the fingers stop.
    const delta = event.deltaX * scale * 0.25;
    const changed = options.move(delta, event);
    if (changed === true) {
      travel += Math.abs(delta);
      const now = performance.now();
      if (travel >= 6 && now - lastHaptic >= 100) {
        bridge()?.haptic?.alignment();
        travel %= 6;
        lastHaptic = now;
      }
    } else if (changed === false) travel = 0;
    timer = setTimeout(finish, 300);
  };
  node.addEventListener('wheel', wheel, { passive: false });
  node.addEventListener('pointerdown', finish, true);
  return { destroy() {
    node.removeEventListener('wheel', wheel);
    node.removeEventListener('pointerdown', finish, true);
    if (timer !== undefined) { clearTimeout(timer); timer = undefined; options.cancel(); }
  } };
}
