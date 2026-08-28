import type { UIPlacement } from './ui-placement';

export interface GhostRect { left: number; top: number; width: number; height: number }

export function ghostRect(placement: UIPlacement, target: GhostRect, before?: GhostRect | null): GhostRect | null {
  if (![target.left, target.top, target.width, target.height].every(Number.isFinite)) return null;
  if (target.width < 32 || target.height < 28) return null;
  const inset = 5;
  const width = target.width - inset * 2;
  const availableHeight = target.height - inset * 2;
  if (placement.kind === 'panel') {
    return { left: target.left + inset, top: target.top + inset, width, height: availableHeight };
  }
  const height = Math.min(148, availableHeight);
  const bottom = target.top + target.height - inset;
  const top = Math.max(target.top + inset, Math.min(before?.top ?? bottom - height, bottom - height));
  return { left: target.left + inset, top, width, height };
}
