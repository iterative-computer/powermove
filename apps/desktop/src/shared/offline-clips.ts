/** Media referenced by an export's composition, including nested compositions. */
export type OfflineClip = { asset: string; name: string };

type Project = Record<string, any>;

export function referencedClips(project: Project, composition: string = project['compId'] ?? project['id']): OfflineClip[] {
  const assets: Record<string, any> = project['assets'] && typeof project['assets'] === 'object' ? project['assets'] : {};
  const comps: Record<string, any> = project['comps'] && typeof project['comps'] === 'object' ? project['comps'] : {};
  const open = typeof project['compId'] === 'string' ? project['compId'] : project['id'];
  const layersOf = (id: string): any[] | null => {
    if (id === open && Array.isArray(project['layers'])) return project['layers'];
    const comp = comps[id];
    return comp && Array.isArray(comp.layers) ? comp.layers : null;
  };
  const out: OfflineClip[] = [];
  const seen = new Set<string>();
  const stack = [composition];
  const visited = new Set<string>();
  while (stack.length) {
    const id = stack.pop()!;
    if (visited.has(id)) continue;
    visited.add(id);
    for (const layer of layersOf(id) ?? []) {
      if (!layer || typeof layer !== 'object') continue;
      const d = layer.d && typeof layer.d === 'object' ? layer.d : {};
      if (layer.type === 'precomp' && d.comp) {
        const target = String(d.comp);
        if (!(target === open && Array.isArray(project['layers']))) stack.push(target);
      } else if ((layer.type === 'video' || layer.type === 'image' || layer.type === 'audio') && d.asset != null) {
        const asset = String(d.asset);
        const record = assets[asset];
        if (!record || seen.has(asset)) continue;
        seen.add(asset);
        out.push({ asset, name: typeof record.name === 'string' && record.name ? record.name : asset });
      }
    }
  }
  return out;
}

export async function offlineClips(project: Project, resolve: (asset: string) => Promise<unknown>): Promise<OfflineClip[]> {
  const clips = referencedClips(project);
  const results = await Promise.all(clips.map(async (clip) => ({ clip, ok: await resolve(clip.asset).catch(() => null) })));
  return results.filter((r) => !r.ok).map((r) => r.clip);
}
