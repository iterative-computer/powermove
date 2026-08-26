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
