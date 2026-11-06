// Kept as the shared selection boundary for custom menus. Commands commit in
// the input event; a visual confirmation must never delay the chosen action.
export const BLINK_MS = 0;

export function prefersReducedMotion(): boolean {
  return typeof matchMedia === 'function' && matchMedia('(prefers-reduced-motion: reduce)').matches;
}

export function blink(_row: HTMLElement, done: () => void): void {
  done();
}
