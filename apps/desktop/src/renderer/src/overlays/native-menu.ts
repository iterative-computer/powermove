import type { NativeMenuItem } from '../../../shared/ipc';
import type { MenuAction, MenuItem } from './types';

/** Menu glyph shortcuts ("⌘⇧Z") → Electron accelerators ("CommandOrControl+Shift+Z"). */
const MODIFIERS: Record<string, string> = { '⌘': 'CommandOrControl', '⇧': 'Shift', '⌥': 'Alt', '⌃': 'Control' };
const KEYS: Record<string, string> = {
  '⌫': 'Backspace', '⌦': 'Delete', '↩': 'Return', '⏎': 'Return', '⎋': 'Escape', '⇥': 'Tab', '␣': 'Space',
  '↑': 'Up', '↓': 'Down', '←': 'Left', '→': 'Right', '⇞': 'PageUp', '⇟': 'PageDown', '↖': 'Home', '↘': 'End'
};

export function acceleratorFor(kb: string | null | undefined): string | undefined {
  if (!kb) return undefined;
  const parts: string[] = [];
  let rest = kb.trim();
  while (rest && MODIFIERS[rest[0]!]) { parts.push(MODIFIERS[rest[0]!]!); rest = rest.slice(1); }
  if (!rest) return undefined;
  const key = KEYS[rest] ?? (/^(F\d{1,2}|[A-Za-z0-9]|[-=\[\];',./`\\])$/.test(rest) ? rest.toUpperCase() : null);
  if (!key) return undefined;
  return [...parts, key].join('+');
}

export type NativeMenuPlan = {
  items: NativeMenuItem[];
  actions: Map<string, MenuAction>;
};

/** Curve tiles draw bezier previews; those menus stay in the DOM. */
export function canRenderNatively(items: MenuItem[]): boolean {
  return items.some((item) => item !== '-' && !('header' in item))
    && !items.some((item) => item !== '-' && !('header' in item) && !!item.curve);
}

export function planNativeMenu(items: MenuItem[]): NativeMenuPlan {
  const actions = new Map<string, MenuAction>();
  const out: NativeMenuItem[] = [];
  items.forEach((item, index) => {
    if (item === '-') { out.push({ type: 'separator' }); return; }
    const id = `m${index}`;
    if ('header' in item) { out.push({ type: 'header', id, label: item.header }); return; }
    actions.set(id, item);
    out.push({
      type: item.on != null ? 'checkbox' : 'normal',
      id,
      label: item.label,
      enabled: !item.disabled,
      checked: !!item.on,
      accelerator: acceleratorFor(item.kb)
    });
  });
  return { items: out, actions };
}
