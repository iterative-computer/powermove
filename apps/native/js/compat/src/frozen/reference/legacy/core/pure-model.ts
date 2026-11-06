/* Selection and media timing of legacy/core/model.ts and media.ts. */
import { resolveContent } from "./content-properties";
import { clipCuesToEnd, rebaseCuesForStart } from "../../captions/model";
import { timelineService } from "./services";
import type { PMRegistry } from "../registry";
export function selectLayers(PM: PMRegistry, ids: any, add = false): void {
  const before = { ...PM.sel, layers: [...PM.sel.layers], keys: [...PM.sel.keys] };
  ids = ([] as any[]).concat(ids).filter(Boolean);
  const timeline = timelineService(PM);
  if (timeline) timeline.keySelectionActive = false;
  PM.sel.layers = add ? [...new Set([...PM.sel.layers, ...ids])] : ids;
  if (!add) PM.sel.keys = [];
  PM.hist?.selection?.(before, PM.sel);
  PM.bus.emit('sel');
  PM.invalidate();
}
export function installMediaTiming(PM: any): void {
PM.MediaTiming = {
  isTimed(L: any) { return !!L && (L.type === 'audio' || L.type === 'video'); },
  rate(L: any) { return L && L.type === 'video' ? Math.max(.0001, Number(resolveContent(PM, L, PM.time).speed) || 1) : 1; },
  trimAtStart(L: any, nextFrom: any) {
    const trim: any = Math.max(0, Number(L ? resolveContent(PM, L, PM.time).trim : 0) || 0);
    return Math.max(0, trim + (Number(nextFrom) - Number(L.from || 0)) * this.rate(L));
  },
  earliestStart(L: any) {
    const trim: any = Math.max(0, Number(L ? resolveContent(PM, L, PM.time).trim : 0) || 0);
    return Number(L.from || 0) - trim / this.rate(L);
  },
  /* Content patch that keeps a clip's content where it was in composition
     time when its In point moves to `nextFrom` (trim-in, a split's tail, the
     timeline's in-edge drag). Media continue at the matching source time;
     captions re-base their layer-time cues. Null for untimed content. */
  startPatch(L: any, nextFrom: any): Record<string, unknown> | null {
    if (this.isTimed(L)) return { trim: this.trimAtStart(L, nextFrom) };
    if (L?.type === 'captions' && Array.isArray(L.d?.cues)) {
      return { cues: rebaseCuesForStart(L.d.cues, Number(nextFrom) - Number(L.from || 0)) };
    }
    return null;
  },
  /* Content patch for the head of a split at composition time `cut`: content
     that now belongs to the tail leaves the head. Null when nothing changes. */
  endPatch(L: any, cut: any): Record<string, unknown> | null {
    if (L?.type === 'captions' && Array.isArray(L.d?.cues)) {
      return { cues: clipCuesToEnd(L.d.cues, Number(cut) - Number(L.from || 0)) };
    }
    return null;
  },
};
}
