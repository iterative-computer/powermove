export const MATTE_MODES = ['alpha', 'alpha-inverted', 'luma', 'luma-inverted', 'vector'];
export function validMatteSources(
  layers: any[],
  layer: any,
  sources: any[] = layer.matteSources ?? [],
  primary = layer.matteSource ?? null,
): boolean {
  const byId = new Map(layers.map((l) => [l.id, l])),
    children = new Map<string, any[]>();
  for (const l of layers) if (l.group) children.set(l.group, [...(children.get(l.group) ?? []), l]);
  const candidate = { ...layer, matteSource: primary, matteSources: sources };
  byId.set(layer.id, candidate);
  const visiting = new Set<string>(),
    done = new Set<string>();
  const visit = (id: string): boolean => {
    if (visiting.has(id)) return false;
    if (done.has(id)) return true;
    const l = byId.get(id);
    if (!l) return false;
    visiting.add(id);
    for (const ref of [l.matteSource, ...(l.matteSources ?? []).map((m: any) => m.source)].filter(Boolean)) {
      const source = byId.get(ref);
      if (!source || ['audio', 'null', 'adjustment'].includes(source.type) || !visit(ref)) return false;
    }
    if (l.type === 'group') for (const child of children.get(id) ?? []) if (!visit(child.id)) return false;
    visiting.delete(id);
    done.add(id);
    return true;
  };
  return visit(layer.id);
}
export function validMatteSource(layers: any[], layer: any, id: string | null): boolean {
  return validMatteSources(layers, layer, layer.matteSources ?? [], id);
}
