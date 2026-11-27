/** JSON only. Nodes/closures stay in the V8 realm; no eval or function source. */
export const VERSION = 1;
export const ROOT = { document: 1, html: 2, head: 3, body: 4, window: 5 } as const;
export const MAX_ID = Number.MAX_SAFE_INTEGER;
export type Session = { panel: string; generation: number };
export type Policy = 'observe' | 'block';
export type PropertyValue = string | number | boolean;
export type Op =
  | {
      op: 'create';
      id: number;
      kind: 'element' | 'text' | 'comment' | 'fragment';
      name: string;
      namespace: string | null;
    }
  | { op: 'insert'; parent: number; id: number; before: number | null }
  | { op: 'remove'; parent: number; id: number }
  | { op: 'attribute'; id: number; name: string; namespace: string | null; value: string | null }
  | { op: 'text'; id: number; value: string }
  | { op: 'property'; id: number; name: string; value: PropertyValue }
  | { op: 'listen'; id: number; event: string; enabled: boolean; policy: Policy }
  | { op: 'focus'; id: number; prevent_scroll: boolean }
  | { op: 'blur'; id: number }
  | { op: 'selection'; id: number; start: number; end: number; direction: string }
  | {
      op: 'range';
      anchor: number | null;
      anchor_offset: number;
      focus: number | null;
      focus_offset: number;
    }
  | { op: 'scroll'; id: number; x: number; y: number; behavior: string }
  | { op: 'measure'; request: number; ids: number[]; styles: string[] }
  | { op: 'observe_resize'; id: number; enabled: boolean };
export type Batch = {
  version: number;
  session: Session;
  seq: number;
  caused_by: number | null;
  ops: Op[];
};
export type Rect = { x: number; y: number; width: number; height: number };
export type LayoutEntry = {
  id: number;
  rect: Rect;
  scroll_left: number;
  scroll_top: number;
  scroll_width: number;
  scroll_height: number;
  client_width: number;
  client_height: number;
  styles: Record<string, string>;
};
export type PageMessage =
  | { kind: 'applied'; session: Session; seq: number }
  | {
      kind: 'event';
      session: Session;
      seq: number;
      applied: number;
      target: number;
      event: string;
      bubbles: boolean;
      cancelable: boolean;
      default_prevented: boolean;
      data: Record<string, unknown>;
    }
  | {
      kind: 'layout';
      session: Session;
      applied: number;
      request: number | null;
      entries: LayoutEntry[];
    }
  | { kind: 'fault'; session: Session; seq: number; message: string };
export function sameSession(a: Session, b: Session): boolean {
  return a.panel === b.panel && a.generation === b.generation;
}
export function validId(id: number): boolean {
  return Number.isSafeInteger(id) && id > 0;
}
export const properties: Record<string, 'string' | 'boolean' | 'number'> = {
  value: 'string',
  checked: 'boolean',
  indeterminate: 'boolean',
  selected: 'boolean',
  disabled: 'boolean',
  multiple: 'boolean',
  readOnly: 'boolean',
  muted: 'boolean',
  open: 'boolean',
  selectedIndex: 'number',
  tabIndex: 'number'
};
