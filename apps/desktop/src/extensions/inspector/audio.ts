/* Whether a precomp carries sound, so the inspector offers its audio level
   and mute only where they do something. Mirrors the mixer's strip rule
   (audio layers with media, videos with a soundtrack, nested precomps at any
   depth); kept local because built-ins may only import the public API. */
const MAX_DEPTH = 8;

type Comps = Record<string, { id?: string; layers?: unknown[] } | undefined> | undefined;

export function precompCarriesAudio(compId: unknown, comps: Comps, depth = 0, seen: ReadonlySet<string> = new Set()): boolean {
  if (typeof compId !== 'string' || !compId || seen.has(compId) || depth > MAX_DEPTH) return false;
  const comp = comps?.[compId];
  if (!comp || !Array.isArray(comp.layers)) return false;
  const next = new Set([...seen, compId]);
  return comp.layers.some((layer: any) => {
    if (!layer || !layer.d) return false;
    if (layer.type === 'audio') return typeof layer.d.asset === 'string' && !!layer.d.asset;
    if (layer.type === 'video') return layer.d.embeddedAudio === true && !!layer.d.asset;
    return layer.type === 'precomp' && precompCarriesAudio(layer.d.comp, comps, depth + 1, next);
  });
}
