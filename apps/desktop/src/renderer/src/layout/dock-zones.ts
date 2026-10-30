import type { PanelPlacement } from './model';

/* After Effects' drop zones. Over a panel, the pointer picks one:
   - its header, or the middle of its body → group: join its tab stack
   - the top or bottom band of its body    → dock above or below it
   AE also offers left and right zones that split a frame side by side;
   Powermove's columns stack panels vertically, so those are left out. */

export type ZoneRect = { left: number; right: number; top: number; bottom: number };
export type ZonePanel = {
  id: string;
  /** Every panel in its tab stack, in order; the panel alone when unstacked. */
  stack: string[];
  rect: ZoneRect;
  /** Null for a headless panel. */
  header: ZoneRect | null;
  /** Tab buttons, or the title for an unstacked panel, left to right. */
  tabs: Array<{ id: string; left: number; right: number }>;
  /** False for panels that cannot show tabs (headless ones, the viewer). */
  groupable: boolean;
};

export type PanelZone =
  | { kind: 'self'; target: string }
  | {
      kind: 'group' | 'beside';
      target: string;
      placement: PanelPlacement;
      /** The part of the target the moved panel will occupy. */
      zone: ZoneRect;
      /** Where its tab will appear in the target's header. */
      tab?: { left: number; top: number; height: number };
    };

/** Share of a body's height at its top and bottom that docks beside it. */
export const EDGE_BAND = 0.3;

const inside = (rect: ZoneRect, x: number, y: number): boolean =>
  x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom;

export function resolvePanelZone(panels: ZonePanel[], x: number, y: number, moving: string[], canGroup: boolean): PanelZone | null {
  const panel = panels.find((item) => inside(item.rect, x, y));
  if (!panel) return null;
  if (panel.stack.every((id) => moving.includes(id))) return { kind: 'self', target: panel.id };
  const group = canGroup && panel.groupable && !!panel.header;
  const { rect, header } = panel;

  /* Over the header the tab lands where the pointer is; over the body it goes last. */
  const grouped = (atPointer: boolean): PanelZone => {
    const after = atPointer ? panel.tabs.find((tab) => x < (tab.left + tab.right) / 2) : undefined;
    const left = after ? after.left : (panel.tabs.at(-1)?.right ?? header!.left + 12);
    return {
      kind: 'group',
      target: panel.id,
      placement: { kind: 'group', target: panel.id, before: after?.id ?? null },
      zone: { ...rect, top: header!.bottom },
      tab: { left, top: header!.top, height: header!.bottom - header!.top }
    };
  };
  const beside = (place: 'above' | 'below'): PanelZone => {
    const mid = (rect.top + rect.bottom) / 2;
    return {
      kind: 'beside',
      target: panel.id,
      placement: { kind: 'beside', target: panel.id, place },
      zone: place === 'above' ? { ...rect, bottom: mid } : { ...rect, top: mid }
    };
  };

  if (group && header && y <= header.bottom) return grouped(true);
  const top = header ? header.bottom : rect.top;
  const relative = (y - top) / Math.max(1, rect.bottom - top);
  if (!group) return beside(relative < 0.5 ? 'above' : 'below');
  if (relative < EDGE_BAND) return beside('above');
  if (relative > 1 - EDGE_BAND) return beside('below');
  return grouped(false);
}
