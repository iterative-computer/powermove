import type { MenuAction, MenuItem } from './types';

/* ── Human Interface Guidelines ─────────────────────────── */

const isAction = (item: MenuItem): item is MenuAction => item !== '-' && !('header' in item);

/** "Cut, Copy, and Paste … may appear unavailable" in a macOS context menu;
    everything else that can't run there is hidden. */
const EDIT_COMMANDS = new Set(['Cut', 'Copy', 'Paste']);

/**
 * A contextual menu, per the macOS HIG: only what applies right now
 * ("Hide unavailable menu items, don't dim them"), and no key equivalents
 * ("Show keyboard shortcuts in your app's main menus, not in context menus").
 * Submenus left with nothing to run go too.
 */
export function contextMenuItems(items: MenuItem[]): MenuItem[] {
  const out: MenuItem[] = [];
  for (const item of items) {
    if (!isAction(item)) { out.push(item); continue; }
    if (item.disabled && !EDIT_COMMANDS.has(item.label)) continue;
    const { kb: _kb, ...rest } = item;
    if (item.submenu) {
      const submenu = tidyMenu(contextMenuItems(item.submenu));
      if (!submenu.some(isAction)) continue;
      out.push({ ...rest, submenu });
      continue;
    }
    out.push(rest);
  }
  return tidyMenu(out);
}

/**
 * Structure every menu the way AppKit draws its own: separators only between
 * groups, a section title only above rows it names, and "a uniform visual
 * treatment across menu items in the same group" — icons on every row of a
 * group or on none of them.
 */
export function tidyMenu(items: MenuItem[]): MenuItem[] {
  const groups: MenuItem[][] = [[]];
  for (const item of items) {
    if (item === '-') { if (groups.at(-1)!.length) groups.push([]); continue; }
    groups.at(-1)!.push(item);
  }
  const out: MenuItem[] = [];
  for (const group of groups) {
    /* A header opens a section; a header with no rows under it is dropped. */
    const kept = group.filter((item, index) => {
      if (isAction(item)) return true;
      const next = group[index + 1];
      return next !== undefined && isAction(next);
    });
    const actions = kept.filter(isAction);
    if (!actions.length) continue;
    const iconed = actions.filter((item) => item.icon).length;
    const uniform = iconed === 0 || iconed === actions.length
      ? kept
      : kept.map((item) => (isAction(item) && item.icon ? { ...item, icon: undefined } : item));
    if (out.length) out.push('-');
    out.push(...uniform);
  }
  return out;
}
