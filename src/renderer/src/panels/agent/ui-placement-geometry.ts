import type { UIPlacement } from './ui-placement';

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
