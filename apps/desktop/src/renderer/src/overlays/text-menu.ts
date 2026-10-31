import type { MenuItem, MenuOptions } from './types';

/** A row of the text-field menu main builds (src/main/text-context-menu.ts). */
export type TextMenuRow = '-' | { id: string; label: string; enabled: boolean };
export type TextMenuRequest = { x: number; y: number; items: TextMenuRow[] };

type OpenMenu = (anchor: HTMLElement, items: MenuItem[], options: MenuOptions) => unknown;

/**
 * Main asks the page to draw the text-field menu (Cut, Copy, Paste, spelling,
 * Look Up) in Powermove's own menu design, and runs the chosen command itself.
 * Opening the menu takes focus from the field, so the field and its selection
 * are put back before main runs Cut, Copy or Paste against them.
 */
export function installTextMenu(open: OpenMenu): () => void {
  const show = (request: TextMenuRequest): Promise<string | null> => new Promise((resolve) => {
    const field = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const selection = document.getSelection();
    const ranges = selection ? Array.from({ length: selection.rangeCount }, (_, index) => selection.getRangeAt(index).cloneRange()) : [];
    let settled = false;
    const done = (id: string | null): void => {
      if (settled) return;
      settled = true;
      if (field?.isConnected) field.focus({ preventScroll: true });
      /* Inputs keep their own selection; editable content needs its range back. */
      if (selection && ranges.length && !(field instanceof HTMLInputElement || field instanceof HTMLTextAreaElement)) {
        selection.removeAllRanges();
        for (const range of ranges) selection.addRange(range);
      }
      resolve(id);
    };
    const items: MenuItem[] = (request?.items ?? []).map((row) => (row === '-'
      ? '-'
      : { label: row.label, disabled: !row.enabled, run: () => done(row.id) }));
    open(document.body, items, { x: Number(request?.x) || 0, y: Number(request?.y) || 0, onClose: () => done(null) });
    /* Nothing applicable to show: the menu never opened. */
    if (!document.querySelector('[data-svelte-overlay-menu]')) done(null);
  });
  const host = globalThis as { __pmTextMenu?: typeof show };
  host.__pmTextMenu = show;
  return () => { if (host.__pmTextMenu === show) delete host.__pmTextMenu; };
}
