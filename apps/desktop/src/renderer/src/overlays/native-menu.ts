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

/* ── icons ──────────────────────────────────────────────── */

/** 2× of the 16pt macOS menu glyph slot. */
export const MENU_ICON_PIXELS = 32;

export type IconRasterizer = (name: string) => Promise<string | undefined>;

const rasterCache = new Map<string, Promise<string | undefined>>();

/** Draw a Powermove icon (256-unit Phosphor markup on currentColor) as a black
    32×32 PNG. AppKit treats it as a template image and tints it itself. */
export function rasterizeIcon(markup: string | undefined, size = MENU_ICON_PIXELS): Promise<string | undefined> {
  if (!markup || typeof document === 'undefined') return Promise.resolve(undefined);
  return new Promise((resolve) => {
    const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256" width="${size}" height="${size}" style="color:#000;fill:currentColor">${markup}</svg>`;
    const image = new Image(size, size);
    const url = URL.createObjectURL(new Blob([svg], { type: 'image/svg+xml' }));
    const done = (value: string | undefined) => { URL.revokeObjectURL(url); resolve(value); };
    image.onload = () => {
      try {
        const canvas = document.createElement('canvas');
        canvas.width = size; canvas.height = size;
        const c = canvas.getContext('2d');
        if (!c) return done(undefined);
        c.drawImage(image, 0, 0, size, size);
        done(canvas.toDataURL('image/png'));
      } catch { done(undefined); }
    };
    image.onerror = () => done(undefined);
    image.src = url;
  });
}

export function iconRasterizer(icons: Record<string, string> | undefined): IconRasterizer {
  return (name) => {
    const markup = icons?.[name];
    if (!markup) return Promise.resolve(undefined);
    let pending = rasterCache.get(name);
    if (!pending) { pending = rasterizeIcon(markup); rasterCache.set(name, pending); }
    return pending;
  };
}

export async function planNativeMenu(items: MenuItem[], icon?: IconRasterizer): Promise<NativeMenuPlan> {
  const actions = new Map<string, MenuAction>();
  const out = await Promise.all(items.map(async (item, index): Promise<NativeMenuItem> => {
    if (item === '-') return { type: 'separator' };
    const id = `m${index}`;
    if ('header' in item) return { type: 'header', id, label: item.header };
    actions.set(id, item);
    return {
      type: item.on != null ? 'checkbox' : 'normal',
      id,
      label: item.label,
      enabled: !item.disabled,
      checked: !!item.on,
      accelerator: acceleratorFor(item.kb),
      icon: item.icon && icon ? await icon(item.icon).catch(() => undefined) : undefined
    };
  }));
  return { items: out, actions };
}
