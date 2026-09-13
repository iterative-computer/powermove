import type { PowermoveAPI } from 'powermove';

function shaderUniforms(code: unknown): Array<{ name: string; min?: number; max?: number }> {
  const result: Array<{ name: string; min?: number; max?: number }> = [];
  const pattern = /uniform\s+(?:float|vec2|vec3|vec4|int|bool)\s+(\w+)\s*;\s*(?:\/\/\s*@param\s*([^\n]*))?/g;
  for (const match of String(code ?? '').matchAll(pattern)) {
    if (/^i(Resolution|Time|GlobalTime|Progress|Frame|Mouse)$/.test(match[1]!)) continue;
    const values = (match[2] ?? '').trim().split(/\s+/).filter(Boolean);
    result.push({
      name: match[1]!,
      ...(values[1] !== undefined && Number.isFinite(Number(values[1])) ? { min: Number(values[1]) } : {}),
      ...(values[2] !== undefined && Number.isFinite(Number(values[2])) ? { max: Number(values[2]) } : {}),
    });
  }
  return result;
}

export function propertyMetadata(api: Pick<PowermoveAPI, 'effects' | 'model'>, layer: any, path: string): any {
  if (api.model.CH?.[path]) return api.model.CH[path];
  if (path.startsWith('x.')) return api.model.layerDefinition?.(layer.d?.definition)?.params?.find((p: any) => p.k === path.slice(2)) ?? {};
  if (path.startsWith('u.')) return shaderUniforms(layer.d?.code).find((property) => property.name === path.slice(2)) ?? {};
  const effect = layer.fx?.find((fx: any) => path.startsWith(`${fx.id}.`));
  if (effect) return api.effects.get(effect.type)?.params.find((property) => property.k === path.slice(effect.id.length + 1)) ?? {};
  const key = path.replace(/^c\./, '');
  const metadata: Record<string, { step?: number; min?: number; max?: number }> = {
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
