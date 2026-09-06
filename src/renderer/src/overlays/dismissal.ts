/* A pointerdown that closes an open menu is spent: it must not also start a
   drag, select, or press whatever happened to be under the pointer. Menus
   mark the event; gesture handlers check it. */
const DISMISSED = Symbol('menu-dismissal');

export function markMenuDismissal(event: Event): void {
  (event as any)[DISMISSED] = true;
}

export function dismissedMenu(event: Event): boolean {
  return Boolean((event as any)[DISMISSED]);
}

/** Spend the rest of a press that dismissed its own dropdown trigger. */
export function consumeMenuTriggerPress(event: PointerEvent, trigger: HTMLElement): void {
  if (trigger === document.body || !trigger.contains(event.target as Node) || event.button !== 0) return;
  event.preventDefault();
  event.stopImmediatePropagation();
  const cleanup = (): void => {
    document.removeEventListener('click', click, true);
    document.removeEventListener('pointerdown', cleanup, true);
    document.removeEventListener('pointercancel', cleanup, true);
  };
  const click = (next: MouseEvent): void => {
    cleanup();
    if (!trigger.contains(next.target as Node)) return;
    next.preventDefault();
    next.stopImmediatePropagation();
  };
  // A cancelled/dragged press must never swallow a later independent click.
  document.addEventListener('click', click, true);
  document.addEventListener('pointerdown', cleanup, true);
  document.addEventListener('pointercancel', cleanup, true);
}
