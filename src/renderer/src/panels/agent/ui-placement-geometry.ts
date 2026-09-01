import type { UIPlacement } from './ui-placement';

export interface GhostRect { left: number; top: number; width: number; height: number }

/** A ghost over a live panel sits just inside it, so it never resizes anything. */
export function ghostRect(target: GhostRect): GhostRect | null {
  if (![target.left, target.top, target.width, target.height].every(Number.isFinite)) return null;
  if (target.width < 32 || target.height < 28) return null;
  const inset = 5;
  return {
    left: target.left + inset,
    top: target.top + inset,
    width: target.width - inset * 2,
    height: target.height - inset * 2
  };
}

/**
 * Where a new panel's ghost belongs among a dock's visible panels, or null when
 * the placement is for somewhere else. The ghost then holds that slot for real,
 * so the dock makes room exactly as it will for the finished panel.
 */
export function ghostSlotIndex(placement: UIPlacement | null | undefined, dockId: string, panelIds: string[]): number | null {
  if (placement?.kind !== 'dock' || placement.id !== dockId) return null;
  const index = placement.beforePanelId ? panelIds.indexOf(placement.beforePanelId) : -1;
  return index < 0 ? panelIds.length : index;
}

/** One skeleton row: a 6px bar plus the gap that follows it. */
const GHOST_ROW_PITCH = 17;

/**
 * How many skeleton rows fill `height`. The ghost is a preview of a panel's
 * shape, so it fills whatever space it was given rather than leaving a fixed
 * handful of bars stranded at the top of a tall panel.
 */
export function ghostRowCount(height: number): number {
  if (!Number.isFinite(height) || height <= 0) return 0;
  return Math.max(0, Math.min(24, Math.floor((height + GHOST_ROW_PITCH - 6) / GHOST_ROW_PITCH)));
}
