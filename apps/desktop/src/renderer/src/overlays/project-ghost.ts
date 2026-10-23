/* While a project opens, the editor body is covered by quiet placeholders in
   the shape of the panels that were on screen (or the default layout on a cold
   boot). The body is inert and busy for that time so the outgoing document
   cannot be edited; the titlebar and status bar stay live. */

/** `PM.projectLoading`, owned by the app: set when a load starts, cleared once
 *  the editor has painted the new project or the load failed. */
export type ProjectLoading = { id: string | null; name: string; generation: number };

export type GhostKind = 'viewer' | 'timeline' | 'list';

export type GhostTile = {
  x: number;
  y: number;
  width: number;
  height: number;
  kind: GhostKind;
  rows: number;
};

/** Loads that finish sooner than this never show a ghost. Mirrored in app.css. */
export const GHOST_DELAY_MS = 150;

const MIN_TILE = 24;

export function readProjectLoading(PM: Record<string, any>): ProjectLoading | null {
  const value = PM.projectLoading;
  if (!value || typeof value !== 'object') return null;
  return {
    id: typeof value.id === 'string' ? value.id : null,
    name: typeof value.name === 'string' ? value.name : '',
    generation: Number(value.generation) || 0
  };
}

export function ghostKind(panelId: string): GhostKind {
  if (panelId === 'viewer') return 'viewer';
  if (panelId === 'timeline') return 'timeline';
  return 'list';
}

/** How many placeholder rows fit below a panel header. */
export function ghostRows(kind: GhostKind, height: number): number {
  if (kind === 'viewer') return 0;
  const pitch = kind === 'timeline' ? 26 : 30;
  return Math.max(0, Math.min(kind === 'timeline' ? 10 : 14, Math.floor((height - 48) / pitch)));
}

type Box = { left: number; top: number; right: number; bottom: number };

function intersect(a: Box, b: Box): Box {
  return {
    left: Math.max(a.left, b.left),
    top: Math.max(a.top, b.top),
    right: Math.min(a.right, b.right),
    bottom: Math.min(a.bottom, b.bottom)
  };
}

/** The visible panels in the body, relative to it. Empty when nothing is laid
 *  out yet, which callers treat as "use the default layout". */
export function measureGhostTiles(body: HTMLElement): GhostTile[] {
  const origin = body.getBoundingClientRect();
  const tiles: GhostTile[] = [];
  for (const panel of body.querySelectorAll<HTMLElement>('.panel')) {
    if (panel.parentElement?.closest('.panel')) continue;
    let rect = intersect(panel.getBoundingClientRect(), origin);
    // Side docks scroll: only the part inside the dock's viewport is on screen.
    const dock = panel.closest('.dock');
    if (dock) rect = intersect(rect, dock.getBoundingClientRect());
    const width = rect.right - rect.left;
    const height = rect.bottom - rect.top;
    if (width < MIN_TILE || height < MIN_TILE) continue;
    const kind = ghostKind(panel.closest('[data-panel-slot]')?.getAttribute('data-panel-slot') ?? '');
    tiles.push({ x: rect.left - origin.left, y: rect.top - origin.top, width, height, kind, rows: ghostRows(kind, height) });
  }
  return tiles;
}

/** Makes the body busy and inert; the returned function puts back exactly
 *  what was there before. */
export function holdBody(body: HTMLElement): () => void {
  const busy = body.getAttribute('aria-busy');
  const inert = body.inert;
  body.setAttribute('aria-busy', 'true');
  body.inert = true;
  return () => {
    if (busy == null) body.removeAttribute('aria-busy');
    else body.setAttribute('aria-busy', busy);
    body.inert = inert;
  };
}
