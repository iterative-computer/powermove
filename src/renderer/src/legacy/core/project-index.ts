import type { PMRegistry } from '../registry';

type CompIndex = {
  generation: number;
  layers: any[];
  layersRef: any[];
  byId: Map<string, any>;
  exactName: Map<string, any>;
  children: Map<string, any[]>;
  byType: Map<string, any[]>;
  intervals: any[];
};

/** Maintained, non-serializable indexes over the editable project graph. */
export function installProjectIndex(PM: PMRegistry): void {
  let generation = 0;
  let comps = new WeakMap<object, CompIndex>();
  let rootProject: any = null;
  let flatLayers: any[] | null = null;
  let keyframes: Map<string, any> | null = null;
  let keyframeLayers: Map<string, any> | null = null;

  const build = (comp: any): CompIndex => {
    const layers = Array.isArray(comp?.layers) ? comp.layers : [];
    const byId = new Map<string, any>();
    const exactName = new Map<string, any>();
    const children = new Map<string, any[]>();
    const byType = new Map<string, any[]>();
    for (const layer of layers) {
      if (typeof layer?.id === 'string') byId.set(layer.id, layer);
      const name = String(layer?.name || '').toLowerCase().trim();
      if (name && !exactName.has(name)) exactName.set(name, layer);
      if (typeof layer?.parent === 'string' && layer.parent) {
        const list = children.get(layer.parent) || [];
        list.push(layer);
        children.set(layer.parent, list);
      }
      const type = String(layer?.type || '');
      const typed = byType.get(type) || [];
      typed.push(layer);
      byType.set(type, typed);
    }
    const intervals = layers.filter((layer: any) => layer?.on !== false)
      .sort((left: any, right: any) => Number(left.from || 0) - Number(right.from || 0));
    const result = { generation, layers, layersRef: layers, byId, exactName, children, byType, intervals };
    if (comp && typeof comp === 'object') comps.set(comp, result);
    return result;
  };

  const ensure = (comp: any): CompIndex => {
    const current = comp && typeof comp === 'object' ? comps.get(comp) : null;
    const layers = Array.isArray(comp?.layers) ? comp.layers : [];
    return current && current.generation === generation
      && current.layersRef === layers && current.layers.length === layers.length
      ? current
      : build(comp);
  };

  const visitProjects = (project: any, action: (comp: any) => void) => {
    const queue = [project];
    const seen = new Set<any>();
    while (queue.length) {
      const comp = queue.shift();
      if (!comp || seen.has(comp)) continue;
      seen.add(comp);
      action(comp);
      queue.push(...Object.values(comp.comps || {}));
    }
  };

  const propertyChannels = (layer: any) => {
    const properties = [...Object.values(layer?.p || {})] as any[];
    for (const effect of layer?.fx || []) properties.push(...Object.values(effect?.p || {}));
    for (const mask of layer?.masks || []) properties.push(...Object.values(mask?.p || {}));
    if (layer?.type === 'shader') properties.push(...Object.values(layer?.d?.uniforms || {}));
    if (layer?.type === 'extension') properties.push(...Object.values(layer?.d?.params || {}));
    for (const field of ['transitionIn', 'transitionOut']) properties.push(...Object.values(layer?.[field]?.p || {}));
    return properties;
  };

  const ensureRoot = () => {
    if (rootProject === PM.proj && flatLayers) return;
    rootProject = PM.proj;
    flatLayers = [];
    keyframes = null;
    keyframeLayers = null;
    visitProjects(PM.proj, comp => flatLayers!.push(...ensure(comp).layers));
  };

  const ensureKeyframes = () => {
    ensureRoot();
    if (keyframes && keyframeLayers) return;
    keyframes = new Map();
    keyframeLayers = new Map();
    for (const layer of flatLayers || []) {
      for (const property of propertyChannels(layer)) {
        for (const key of property?.kf || []) {
          if (typeof key?.i !== 'string') continue;
          keyframes.set(key.i, key);
          keyframeLayers.set(key.i, layer);
        }
      }
    }
  };

  const API: any = {
    invalidate() {
      generation++;
      comps = new WeakMap();
      rootProject = null;
      flatLayers = null;
      keyframes = null;
      keyframeLayers = null;
    },
    invalidateKeyframes() {
      keyframes = null;
      keyframeLayers = null;
    },
    byId(id: any, comp: any = PM.curComp?.() || PM.proj) {
      return typeof id === 'string' ? ensure(comp).byId.get(id) || null : null;
    },
    byName(name: any, comp: any = PM.proj) {
      const query = String(name || '').toLowerCase().trim();
      if (!query) return null;
      const index = ensure(comp);
      return index.exactName.get(query)
        || index.layers.find(layer => String(layer?.name || '').toLowerCase().includes(query))
        || null;
    },
    allLayers() {
      ensureRoot();
      return flatLayers!;
    },
    selectedLayers(ids: any[]) {
      const index = ensure(PM.proj);
      return (ids || []).map(id => index.byId.get(id)).filter(Boolean);
    },
    layersOfType(type: string, comp: any = PM.proj) {
      return ensure(comp).byType.get(type) || [];
    },
    parentOptions(layer: any) {
      const index = ensure(PM.proj);
      const blocked = new Set<any>([layer?.id]);
      const queue = [...(index.children.get(layer?.id) || [])];
      while (queue.length) {
        const child = queue.shift();
        if (!child || blocked.has(child.id)) continue;
        blocked.add(child.id);
        queue.push(...(index.children.get(child.id) || []));
      }
      return index.layers
        .filter(candidate => !blocked.has(candidate.id))
        .map(candidate => ({ v: candidate.id, label: String(candidate.name) }));
    },
    activeAt(time: number, comp: any = PM.curComp?.() || PM.proj) {
      const intervals = ensure(comp).intervals;
      let low = 0, high = intervals.length;
      while (low < high) {
        const middle = (low + high) >>> 1;
        if (Number(intervals[middle]?.from || 0) <= time) low = middle + 1;
        else high = middle;
      }
      return intervals.slice(0, low).filter(layer => Number(layer.from || 0) <= time
        && time < Number(layer.from || 0) + Math.max(0, Number(layer.dur || 0)));
    },
    keyframe(id: string) {
      ensureKeyframes();
      return keyframes!.get(id) || null;
    },
    layerForKeyframe(id: string) {
      ensureKeyframes();
      return keyframeLayers!.get(id) || null;
    },
    stats() {
      ensureRoot();
      ensureKeyframes();
      return { layers: flatLayers!.length, keyframes: keyframes!.size, generation };
    },
  };

  PM.ProjectIndex = API;
}
