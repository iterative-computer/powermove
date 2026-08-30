import type { PanelDefinition } from '../kernel/api';

export interface PanelPreviewSize {
  width: number;
  height: number;
}

const DEFAULT_WIDTH = 360;
const DEFAULT_HEIGHT = 320;
const MIN_HEIGHT = 180;
const MAX_HEIGHT = 520;

export function panelBelongsInLibrary(panel: Pick<PanelDefinition, 'library'>): boolean {
  return panel.library !== false;
}

/**
 * Resolve a stable presentation size from the panel definition alone.
 * Workspace dock geometry is deliberately not accepted here: opening, moving,
 * or resizing a panel must not change its Library preview.
 */
export function panelPreviewSize(panel: Pick<PanelDefinition, 'library' | 'size'>): PanelPreviewSize {
  if (panel.library) {
    return {
      width: Math.max(1, Math.round(panel.library.width)),
      height: Math.max(1, Math.round(panel.library.height))
    };
  }
  const requestedHeight = Number.isFinite(panel.size) ? Number(panel.size) : DEFAULT_HEIGHT;
  return {
    width: DEFAULT_WIDTH,
    height: Math.min(MAX_HEIGHT, Math.max(MIN_HEIGHT, Math.round(requestedHeight)))
  };
}
