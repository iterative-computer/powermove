/** Only built-in artwork with no temporal inputs can share captured pixels.
 * Media, custom renderers, effects and animation keep their per-frame path. */
export function staticExportPlan(project: any, motionBlur: boolean) {
  const edges = new Set<number>();
  let blur = false;
  const constant = (value: any, seen = new Set<object>()): boolean => {
    if (!value || typeof value !== 'object' || seen.has(value)) return true;
    seen.add(value);
    if (value.expr || Array.isArray(value.kf) && value.kf.length > 1) return false;
    return Object.values(value).every(child => constant(child, seen));
  };
  if (!constant(project.backgroundFill)) return null;
  for (const layer of project.layers || []) {
    if (layer.type === 'audio') continue;
    if (!['solid', 'shape', 'text', 'null', 'group'].includes(layer.type)
        || layer.fx?.length || layer.transitionIn || layer.transitionOut
        || layer.threeD || layer.matteSource || layer.masks?.length
        || layer.d?.animators?.length || !constant(layer)) return null;
    const from = Number(layer.from), end = from + Number(layer.dur);
    if (!Number.isFinite(from) || !Number.isFinite(end)) return null;
    edges.add(from); edges.add(end);
    blur ||= !!layer.mblur;
  }
  const boundaries = [...edges].sort((a, b) => a - b);
  const radius = motionBlur && blur ? (project.shutter || .5) / Math.max(1, project.fps) : 0;
  return (time: number): string => {
    let interval = 0;
    for (const edge of boundaries) {
      // Blur can sample both sides of a layer entrance/exit. Never retain
      // those frames; ordinary hard boundaries belong to the new interval.
      if (radius && Math.abs(time - edge) <= radius + 2e-6) return `frame:${time}`;
      if (edge <= time) interval++; else break;
    }
    return `still:${interval}`;
  };
}
