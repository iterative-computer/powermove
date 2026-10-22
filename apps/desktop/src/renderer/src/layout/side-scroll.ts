/* Wheel routing for the side columns. A side column and the panels inside it
   both scroll, and the browser latches a wheel gesture to the scroller it
   started on: once a panel's list hits its end, the rest of that swipe went
   nowhere, and the column only moved when the pointer sat on a header or a
   gap. Route each wheel event instead: the innermost element that can still
   move in that direction takes it, and otherwise the column does. */

export const SIDE_DOCKS = new Set(['left', 'right']);

const wheelPixels = (event: WheelEvent, page: number): number =>
  event.deltaY * (event.deltaMode === WheelEvent.DOM_DELTA_PAGE ? page
    : event.deltaMode === WheelEvent.DOM_DELTA_LINE ? 16 : 1);

/** Whether `element` can still scroll vertically in the direction of `delta`. */
export function canScroll(element: Element, delta: number): boolean {
  const remaining = element.scrollHeight - element.clientHeight;
  if (remaining <= 1) return false;
  return delta < 0 ? element.scrollTop > 0.5 : element.scrollTop < remaining - 0.5;
}

function scrollsVertically(element: Element): boolean {
  const overflow = getComputedStyle(element).overflowY;
  return overflow === 'auto' || overflow === 'scroll';
}

/** Scroll a side column by a wheel event that no inner element wants. */
export function scrollSideDock(dock: HTMLElement, event: WheelEvent): boolean {
  if (event.defaultPrevented || event.ctrlKey || event.metaKey || event.shiftKey || !event.deltaY) return false;
  const delta = wheelPixels(event, dock.clientHeight);
  if (!canScroll(dock, delta)) return false;
  event.preventDefault();
  dock.scrollTop += delta;
  return true;
}

/** Wheel handler for a side column: inner scrollers first, then the column. */
export function routeSideWheel(dock: HTMLElement, event: WheelEvent): void {
  if (event.defaultPrevented || event.ctrlKey || event.metaKey || event.shiftKey || !event.deltaY) return;
  const delta = wheelPixels(event, dock.clientHeight);
  for (let node = event.target instanceof Element ? event.target : null; node && node !== dock; node = node.parentElement) {
    // Let the innermost scroller with room take the event natively.
    if (scrollsVertically(node) && canScroll(node, delta)) return;
  }
  scrollSideDock(dock, event);
}

/** The side column whose rows a point beside it (a gutter or divider) belongs to. */
export function sideDockBeside(x: number, y: number, docks: Iterable<HTMLElement>): HTMLElement | null {
  for (const dock of docks) {
    if (!dock.clientHeight) continue;
    const rect = dock.getBoundingClientRect();
    if (y < rect.top || y >= rect.bottom) continue;
    const side = dock.dataset.dock;
    if (side === 'left' ? x < rect.left : side === 'right' ? x >= rect.right : false) return dock;
  }
  return null;
}
