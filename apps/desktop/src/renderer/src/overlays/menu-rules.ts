import type { MenuAction, MenuItem } from './types';

/* ── menu structure ─────────────────────────────────────── */

const isAction = (item: MenuItem): item is MenuAction => item !== '-' && !('header' in item);

/**
 * Structure every menu: separators only between groups, a section title only
 * above rows it names, and no icon repeated within one menu: when two rows
 * share an icon (Set Work Area Start and End), only the first keeps it, so
 * each icon still points at one action.
 */
export function tidyMenu(items: MenuItem[]): MenuItem[] {
  const groups: MenuItem[][] = [[]];
  for (const item of items) {
    if (item === '-') { if (groups.at(-1)!.length) groups.push([]); continue; }
    groups.at(-1)!.push(item);
  }
  const out: MenuItem[] = [];
  const seen = new Set<string>();
  for (const group of groups) {
    /* A header opens a section; a header with no rows under it is dropped. */
    const kept = group.filter((item, index) => {
      if (isAction(item)) return true;
      const next = group[index + 1];
      return next !== undefined && isAction(next);
    });
    if (!kept.some(isAction)) continue;
    if (out.length) out.push('-');
    for (const raw of kept) {
      /* Each submenu is a menu of its own, tidied the same way. */
      const item = isAction(raw) && raw.submenu ? { ...raw, submenu: tidyMenu(raw.submenu) } : raw;
      if (!isAction(item) || !item.icon) { out.push(item); continue; }
      if (seen.has(item.icon)) { out.push({ ...item, icon: undefined }); continue; }
      seen.add(item.icon);
      out.push(item);
    }
  }
  return out;
}
