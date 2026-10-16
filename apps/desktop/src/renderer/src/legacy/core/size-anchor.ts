import { resolveContent } from './content-properties';
import { anchorKernel, commandFields, fontAnchorOffset } from './font-anchor';

const sizedTypes = new Set(['shape', 'solid', 'image', 'video', 'shader', 'extension', 'precomp']);

/** Native content bounds, excluding stroke and raster padding. */
export function sizeBounds(PM: any, layer: any, time: number) {
  if (!layer || !sizedTypes.has(layer.type) || layer.type === 'shape' && layer.d.paths?.length) return null;
  const d = resolveContent(PM, layer, time);
  const w = Number(d.w || PM.proj.w), h = Number(d.h || PM.proj.h);
  if (![w, h].every(value => Number.isFinite(value) && value > 0)) return null;
  const centered = ['shape', 'image', 'video'].includes(layer.type);
  return { x0: centered ? -w / 2 : 0, y0: centered ? -h / 2 : 0,
    x1: centered ? w / 2 : w, y1: centered ? h / 2 : h };
}

export function sizeAnchorOffset(PM: any, layer: any, time: number) {
  if (!layer.d?.sizeAnchorBounds) return { x: 0, y: 0 };
  return fontAnchorOffset(layer.d.sizeAnchorBounds, sizeBounds(PM, layer, time),
    PM.ev(layer, 'anchor.x', time), PM.ev(layer, 'anchor.y', time));
}

export function captureSizeAnchor(PM: any, layer: any, command: any) {
  if (!layer || layer.d?.sizeAnchorBounds) return;
  const fields = commandFields(command);
  if (!fields.some(key => key === 'w' || key === 'h')) return;
  const time = command.time ?? PM.time;
  const legacy = () => {
    const bounds = sizeBounds(PM, layer, time);
    if (bounds) layer.d.sizeAnchorBounds = bounds;
  };
  const kernel = anchorKernel(PM, layer);
  if (kernel) kernel.edits.edit('captureSizeAnchor', { layer: kernel.index, fields, t: time }, legacy, () => undefined);
  else legacy();
}

/** The native size bounds shifted by the size anchor offset (what the artwork
 *  occupies), before an anchor edit; null when nothing is recorded. */
export function sizeAnchorReference(PM: any, layer: any, time: number): any {
  const legacy = () => {
    const native = sizeBounds(PM, layer, time);
    const offset = native ? sizeAnchorOffset(PM, layer, time) : null;
    return native && offset ? { x0: native.x0 + offset.x, x1: native.x1 + offset.x, y0: native.y0 + offset.y, y1: native.y1 + offset.y } : null;
  };
  const edits = PM.EngineDoc?.edits;
  return edits ? edits.anchorReference(layer, 'size', time, legacy) : legacy();
}
