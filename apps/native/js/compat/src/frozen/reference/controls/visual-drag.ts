import { EditGesture, type EditBinding } from './gesture';
import type { PowermoveAPI } from '../kernel/api';

/** One drag is one undo step. Escape, pointer cancellation and unmount restore it. */
export function visualDrag(node: HTMLElement, options: {
  api: PowermoveAPI; edit: EditBinding; get: () => unknown;
  value: (event: PointerEvent, bounds: DOMRect) => unknown;
  bounds?: (node: HTMLElement) => DOMRect;
}) {
  let active: { gesture: EditGesture; before: unknown; bounds: DOMRect; pointer: number } | null = null;
  const move = (event: PointerEvent) => {
    if (!active || event.pointerId !== active.pointer) return;
    active.gesture.write(options.value(event, active.bounds));
  };
  const finish = (cancel = false) => {
    if (!active) return;
    const { gesture, before } = active;
    active = null;
    if (cancel) {
      if (options.edit.mode === 'local') gesture.write(before);
      gesture.cancel();
    } else gesture.commit();
    window.removeEventListener('pointermove', move);
    window.removeEventListener('pointerup', up);
    window.removeEventListener('pointercancel', abort);
    window.removeEventListener('keydown', key, true);
  };
  const up = (event: PointerEvent) => { if (active?.pointer === event.pointerId) { move(event); finish(); } };
  const abort = () => finish(true);
  const key = (event: KeyboardEvent) => { if (event.key === 'Escape' && active) { event.preventDefault(); event.stopPropagation(); finish(true); } };
  const down = (event: PointerEvent) => {
    if (event.button !== 0 || active) return;
    event.preventDefault(); event.stopPropagation(); node.focus();
    const gesture = new EditGesture(options.api, options.edit);
    active = { gesture, before: options.get(), bounds: options.bounds?.(node) ?? node.getBoundingClientRect(), pointer: event.pointerId };
    gesture.begin(); move(event);
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', up);
    window.addEventListener('pointercancel', abort);
    window.addEventListener('keydown', key, true);
  };
  // Capture makes Escape win over the inspector's shortcuts.
  const removeKey = () => window.removeEventListener('keydown', key, true);
  node.addEventListener('pointerdown', down);
  return { update(next: typeof options) { options = next; }, destroy() { finish(true); removeKey(); node.removeEventListener('pointerdown', down); } };
}
