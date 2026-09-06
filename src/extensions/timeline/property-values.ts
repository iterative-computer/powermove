export function propertyMetadata(PM: any, layer: any, path: string): any {
  if (PM.CH?.[path]) return PM.CH[path];
  if (path.startsWith('x.')) return PM.layerDefinition?.(layer.d?.definition)?.params?.find((p: any) => p.k === path.slice(2)) ?? {};
  if (path.startsWith('u.')) return PM.UIState?.getShaderMeta?.(layer)?.udefs?.find((p: any) => p.name === path.slice(2)) ?? {};
  const effect = layer.fx?.find((fx: any) => path.startsWith(`${fx.id}.`));
  if (effect) return PM.FX?.[effect.type]?.params?.find((p: any) => p.k === path.slice(effect.id.length + 1)) ?? {};
  const key = path.replace(/^c\./, '');
  const metadata: Record<string, any> = {
    w: { step: 1, min: 1 }, h: { step: 1, min: 1 }, size: { step: 1, min: 4 },
    tracking: { step: .5 }, leading: { step: .02 }, radius: { step: 1, min: 0 },
    stroke: { step: .5, min: 0 }, points: { step: 1, min: 3, max: 24 },
    weight: { step: 1, min: 1, max: 1000 }, speed: { step: .05, min: .05 },
    gain: { step: .05, min: 0, max: 4 }, trim: { step: .05, min: 0 },
    fadeIn: { step: .05, min: 0 }, fadeOut: { step: .05, min: 0 }
  };
  return metadata[key] ?? {};
}

export function draggedPropertyValue(start: number, delta: number, meta: any, fine = false, fast = false): number {
  const step = meta.step ?? .1;
  const value = start + delta * step * (fine ? .1 : fast ? 10 : 1);
  return Math.max(meta.min ?? -Infinity, Math.min(meta.max ?? Infinity, Math.round(value * 1e6) / 1e6));
}
