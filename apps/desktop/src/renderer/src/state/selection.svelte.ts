/* Selection state — small, id-only, safe as deep $state. */
import type { Selection } from '../runtime/ports';

export const sel: Selection = $state({ layers: [], keys: [], chan: null });

export function setSelection(next: Selection): void {
  sel.layers = [...next.layers];
  sel.keys = [...next.keys];
  sel.chan = next.chan;
}
