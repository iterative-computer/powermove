/* How a page arrives, shared by the Store, the Library and the home screen so
   moving around the app keeps one gesture: going deeper slides in from the
   right and going back from the left. Sibling sections change immediately;
   content that settles within a page only fades. */
import { cubicOut } from 'svelte/easing';

export const PAGE_MS = 200;
export const PAGE_SLIDE = 24;
export const SETTLE_MS = 180;
/** svelte/easing's cubicOut as a CSS curve, for WAAPI and stylesheets. */
export const PAGE_EASE_CSS = 'cubic-bezier(.33,1,.68,1)';

/** -1 back, 0 sideways, 1 deeper. */
export type PageDirection = -1 | 0 | 1;

export function reducedMotion(): boolean {
  return typeof matchMedia === 'function' && matchMedia('(prefers-reduced-motion: reduce)').matches;
}

/** Parameters for svelte/transition's `fly`. */
export function pageFly(direction: PageDirection, reduced = reducedMotion()): { x?: number; y?: number; duration: number; easing?: (t: number) => number } {
  if (reduced || direction === 0) return { duration: 0 };
  return { x: direction * PAGE_SLIDE, duration: PAGE_MS, easing: cubicOut };
}

/** Parameters for svelte/transition's `fade`. */
export function pageSettle(reduced = reducedMotion()): { duration: number; easing?: (t: number) => number } {
  return reduced ? { duration: 0 } : { duration: SETTLE_MS, easing: cubicOut };
}

/** The same arrival for DOM that Svelte does not own. */
export function playPageIn(element: Element, direction: PageDirection = 0): Animation | null {
  if (direction === 0 || reducedMotion() || typeof (element as HTMLElement).animate !== 'function') return null;
  const from = `translateX(${direction * PAGE_SLIDE}px)`;
  return (element as HTMLElement).animate(
    [{ opacity: 0, transform: from }, { opacity: 1, transform: 'none' }],
    { duration: PAGE_MS, easing: PAGE_EASE_CSS }
  );
}
