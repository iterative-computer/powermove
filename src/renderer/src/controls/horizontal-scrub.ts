/** Group a horizontal trackpad gesture into one edit, leaving vertical scrolling alone. */
export function horizontalScrub(node: HTMLElement, options: {
  begin: () => void; move: (delta: number, event: WheelEvent) => void;
  commit: () => void; cancel: () => void; enabled?: () => boolean;
}) {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const finish = () => { if (timer === undefined) return; clearTimeout(timer); timer = undefined; options.commit(); };
  const wheel = (event: WheelEvent) => {
    if (options.enabled?.() === false || event.ctrlKey || !event.deltaX || Math.abs(event.deltaX) <= Math.abs(event.deltaY)) return;
    event.preventDefault(); event.stopPropagation();
    if (timer === undefined) options.begin();
    else clearTimeout(timer);
    const scale = event.deltaMode === 1 ? 16 : event.deltaMode === 2 ? node.clientWidth : 1;
    options.move(event.deltaX * scale, event);
    timer = setTimeout(finish, 180);
  };
  node.addEventListener('wheel', wheel, { passive: false });
  node.addEventListener('pointerdown', finish, true);
  return { destroy() {
    node.removeEventListener('wheel', wheel);
    node.removeEventListener('pointerdown', finish, true);
    if (timer !== undefined) { clearTimeout(timer); timer = undefined; options.cancel(); }
  } };
}
