/* Shared state between the Effects panel's header (search toggle, mounted by
   the panel header hook) and its body (category rail + list). */
export const fxBrowser = $state({
  query: '',
  searchOpen: false,
  /** 'all', 'recent', or an effect group name. */
  category: 'all' as string
});

const RECENT_KEY = 'pm.fx.recent';
const RECENT_MAX = 8;

export function readRecent(): string[] {
  try {
    const raw = window.localStorage?.getItem(RECENT_KEY);
    const list = raw ? JSON.parse(raw) : [];
    return Array.isArray(list) ? list.filter((id) => typeof id === 'string') : [];
  } catch {
    return [];
  }
}

export function pushRecent(id: string): string[] {
  const next = [id, ...readRecent().filter((other) => other !== id)].slice(0, RECENT_MAX);
  try { window.localStorage?.setItem(RECENT_KEY, JSON.stringify(next)); } catch { /* storage unavailable */ }
  return next;
}
