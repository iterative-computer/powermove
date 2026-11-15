/** Registration-time checks mirror pm-fx-defs::handles. UI declarations stay
 * on the definition; render snapshots do not need them. */
export function validateViewerHandles(def: any): void {
  const handles = def?.viewerHandles;
  if (handles === undefined) return;
  if (!Array.isArray(handles) || handles.length > 32)
    throw new Error('viewerHandles: expected at most 32 controls');
  const param = (key: unknown, type = 'number') => {
    if (typeof key !== 'string' || !def.params?.some((p: any) => p.k === key && (p.type ?? 'number') === type))
      throw new Error(`viewerHandles: ${String(key)} must bind a ${type} parameter`);
  };
  const point = (p: any) => {
    if (!p || Object.keys(p).some(k => !['x', 'y'].includes(k))) throw new Error('viewerHandles: invalid point');
    param(p.x); param(p.y);
    if (p.x === p.y) throw new Error('viewerHandles: point axes must differ');
  };
  for (const h of handles) {
    if (!h || typeof h !== 'object') throw new Error('viewerHandles: invalid control');
    let fields = ['kind'];
    switch (h.kind) {
      case 'point': fields.push('point'); point(h.point); break;
      case 'line': fields.push('start', 'end', 'stops', 'radial'); point(h.start); point(h.end); if (h.stops != null) param(h.stops, 'gradientStops'); if (h.radial != null) param(h.radial); break;
      case 'rect': fields.push('start', 'end'); point(h.start); point(h.end); break;
      case 'angle': fields.push('center', 'angle'); point(h.center); param(h.angle); break;
      case 'radius': fields.push('center', 'radius'); point(h.center); param(h.radius); break;
      default: throw new Error('viewerHandles: unknown kind');
    }
    if (Object.keys(h).some(k => !fields.includes(k))) throw new Error('viewerHandles: unknown field');
  }
}
