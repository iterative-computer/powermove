/* After Effects compositions.
 *
 * A project holds any number of compositions. Exactly one of them — the one
 * open in the Timeline and Composition viewer — lives at the top level of the
 * project object (`layers`, `w`, `h`, `fps`, … plus `compId` / `compName`);
 * every other composition lives in the flat `proj.comps` map. Opening a
 * composition swaps it with the top level. Everything that edits "the
 * composition" through `PM.proj` therefore edits whichever comp is open, the
 * same way every AE panel follows the active composition.
 *
 * Undo history is stored in a root-independent ("canonical") form: a patch on
 * the open comp's `layers` is recorded as `['comps', <compId>, 'layers', …]`
 * and translated back to whichever comp is open when it is undone. Switching
 * compositions is therefore never an undo step, just as in AE. */
import type { PMRegistry } from '../registry';
import type { Patch, PathPart } from '../../../../shared/patch';

/** Fields that belong to a composition rather than to the project. */
export const COMP_FIELDS = ['w', 'h', 'fps', 'dur', 'bg', 'backgroundFill', 'layers', 'markers', 'work', 'shutter', 'audioGain', 'render3d'] as const;
/** Composition fields that may be absent (absent = default). When the open
 * composition changes they must be cleared rather than inherited, or one
 * composition's value would leak into the next. */
const OPTIONAL_COMP_FIELDS: ReadonlySet<string> = new Set(['audioGain', 'render3d']);
/** Top-level project key → key on a stored composition. */
const ROOT_TO_COMP: Record<string, string> = {
  compId: 'id', compName: 'name',
  ...Object.fromEntries(COMP_FIELDS.map(key => [key, key])),
};
const COMP_TO_ROOT: Record<string, string> = Object.fromEntries(Object.entries(ROOT_TO_COMP).map(([root, comp]) => [comp, root]));

const isRecord = (value: any): value is Record<string, any> => !!value && typeof value === 'object' && !Array.isArray(value);
const json = (value: any) => JSON.stringify(value);

/** The open composition as a stand-alone composition record. */
export function rootComp(project: any): any {
  const comp: any = { id: project.compId, name: project.compName };
  for (const key of COMP_FIELDS) if (Object.hasOwn(project, key)) comp[key] = project[key];
  return comp;
}

/** Return a project whose open composition is `id`. Pure: the input is not mutated. */
export function rotateProject(project: any, id: string): any {
  if (!project || !id || project.compId === id) return project;
  const target = project.comps?.[id];
  if (!isRecord(target)) return project;
  const comps: Record<string, any> = {};
  for (const [key, comp] of Object.entries(project.comps || {})) if (key !== id) comps[key] = comp;
  if (project.compId) comps[project.compId] = rootComp(project);
  const next: any = { ...project, comps, compId: id, compName: target.name || 'Comp' };
  for (const key of COMP_FIELDS) {
    if (Object.hasOwn(target, key)) next[key] = target[key];
    else if (OPTIONAL_COMP_FIELDS.has(key)) delete next[key];
  }
  next.layers = Array.isArray(next.layers) ? next.layers : [];
  next.markers = Array.isArray(next.markers) ? next.markers : [];
  next.work = Array.isArray(next.work) && next.work.length === 2 ? next.work : [0, next.dur];
  return next;
}

/** Record a patch path independent of which composition is open. */
export function canonicalPath(path: PathPart[], rootId: string | undefined): PathPart[] {
  if (!rootId || !path.length) return path;
  const key = path[0];
  if (typeof key === 'string' && Object.hasOwn(ROOT_TO_COMP, key)) return ['comps', rootId, ROOT_TO_COMP[key]!, ...path.slice(1)];
  return path;
}

/**
 * Canonicalise a forward/backward pair recorded while `rootId` was open.
 * Whole-map `['comps']` patches are split per composition: the map never
 * contains the open comp, so replaying it wholesale after a switch would drop
 * a composition.
 */
export function canonicalizePatches(forward: Patch[], backward: Patch[], rootId: string | undefined): { forward: Patch[]; backward: Patch[] } {
  if (!rootId) return { forward, backward };
  const f: Patch[] = [], b: Patch[] = [];
  const count = Math.max(forward.length, backward.length);
  for (let index = 0; index < count; index++) {
    const fp = forward[index], bp = backward[index];
    const whole = (patch?: Patch) => !!patch && patch.path.length === 1 && patch.path[0] === 'comps';
    if (whole(fp) && whole(bp)) {
      const after = fp!.exists && isRecord(fp!.value) ? fp!.value : {};
      const before = bp!.exists && isRecord(bp!.value) ? bp!.value : {};
      for (const id of new Set([...Object.keys(after), ...Object.keys(before)])) {
        if (id === rootId) continue;
        const has = Object.hasOwn(after, id), had = Object.hasOwn(before, id);
        if (has && had && json(after[id]) === json(before[id])) continue;
        f.push(has ? { path: ['comps', id], exists: true, value: after[id] } : { path: ['comps', id], exists: false });
        b.push(had ? { path: ['comps', id], exists: true, value: before[id] } : { path: ['comps', id], exists: false });
      }
      continue;
    }
    if (fp) f.push({ ...fp, path: canonicalPath(fp.path, rootId) });
    if (bp) b.push({ ...bp, path: canonicalPath(bp.path, rootId) });
  }
  return { forward: f, backward: b };
}

/** Translate canonical patches onto a project whose open composition is `project.compId`. */
export function realizePatches(patches: Patch[], project: any): Patch[] {
  const rootId = project?.compId;
  const out: Patch[] = [];
  for (const patch of patches) {
    const path = patch.path;
    if (!path.length) {
      out.push(patch.exists && isRecord(patch.value) && rootId && patch.value.compId !== rootId && patch.value.comps?.[rootId]
        ? { path: [], exists: true, value: rotateProject(patch.value, rootId) } : patch);
      continue;
    }
    if (rootId && path[0] === 'comps' && path[1] === rootId) {
      if (path.length === 2) {
        if (patch.exists && isRecord(patch.value)) {
          for (const [key, value] of Object.entries(patch.value)) {
            const rootKey = COMP_TO_ROOT[key];
            if (rootKey && rootKey !== 'compId') out.push({ path: [rootKey], exists: true, value });
          }
          for (const key of OPTIONAL_COMP_FIELDS) if (!Object.hasOwn(patch.value, key)) out.push({ path: [key], exists: false });
        }
        continue;
      }
      const rootKey = COMP_TO_ROOT[String(path[2])];
      if (rootKey) out.push({ ...patch, path: [rootKey, ...path.slice(3)] });
      continue;
    }
    out.push(patch);
  }
  return out;
}

/**
 * Bring a loaded project into the composition shape: the open comp gets an id
 * and name, and nested `comp.comps` maps from older files are flattened into
 * the project's one `comps` map.
 */
export function normalizeCompositions(project: any, uid: (prefix: string) => string): any {
  if (!project || typeof project !== 'object') return project;
  if (typeof project.compId !== 'string' || !project.compId) project.compId = uid('C');
  if (typeof project.compName !== 'string' || !project.compName.trim()) project.compName = String(project.name || 'Comp 1');
  const flat: Record<string, any> = {};
  const visit = (comps: any) => {
    if (!isRecord(comps)) return;
    for (const [id, comp] of Object.entries(comps)) {
      if (!isRecord(comp) || id === project.compId) continue;
      if (!Object.hasOwn(flat, id)) flat[id] = comp;
      const nested = comp.comps;
      comp.comps = {};
      visit(nested);
      comp.id = id;
    }
  };
  visit(project.comps);
  project.comps = flat;
  return project;
}

export function install(PM: PMRegistry): void {
  /* Per-composition view state (AE keeps each comp's current time and layer
     selection). Session-only, keyed by project id + comp id. */
  const views = new Map<string, { time: number; layers: string[]; scrollT?: number; scrollY?: number; pps?: number }>();
  /* Timeline tabs, per project. */
  const tabs = new Map<string, string[]>();
  const viewKey = (id: string) => `${PM.proj?.id}:${id}`;
  const transact = (label: string, fn: () => void) => PM.hist?.do ? PM.hist.do(label, fn) : fn();

  const comps = () => PM.proj?.comps || {};
  const isRoot = (id: string) => !!PM.proj && PM.proj.compId === id;
  const exists = (id: string) => isRoot(id) || isRecord(comps()[id]);
  /** Settings/layers of any composition, the open one included. */
  const get = (id: string): any => isRoot(id) ? rootComp(PM.proj) : comps()[id] || null;
  const layersOf = (id: string): any[] => isRoot(id) ? PM.proj.layers : comps()[id]?.layers || [];
  const allIds = () => PM.proj ? [PM.proj.compId, ...Object.keys(comps())] : [];

  const naturalCompare = (a: string, b: string) => a.localeCompare(b, undefined, { numeric: true, sensitivity: 'base' });

  const uniqueName = (base: string, except?: string) => {
    const taken = new Set(allIds().filter(id => id !== except).map(id => String(get(id)?.name || '')));
    if (!taken.has(base)) return base;
    const stem = base.replace(/\s+\d+$/, '');
    for (let n = 2; ; n++) if (!taken.has(`${stem} ${n}`)) return `${stem} ${n}`;
  };
  /** AE numbering: “Comp 1”, “Comp 2”, … / “Pre-comp 1”, … */
  const nextName = (stem: string) => {
    const taken = new Set(allIds().map(id => String(get(id)?.name || '')));
    for (let n = 1; ; n++) if (!taken.has(`${stem} ${n}`)) return `${stem} ${n}`;
  };

  /** Composition ids a comp nests, directly. */
  const children = (id: string) => [...new Set(layersOf(id)
    .filter((layer: any) => layer.type === 'precomp' && layer.d?.comp && exists(layer.d.comp))
    .map((layer: any) => layer.d.comp as string))];
  /** Does composition `ancestor` contain `id` anywhere below it (or equal it)? */
  const contains = (ancestor: string, id: string): boolean => {
    const seen = new Set<string>(), queue = [ancestor];
    while (queue.length) {
      const next = queue.shift()!;
      if (next === id) return true;
      if (seen.has(next)) continue;
      seen.add(next);
      queue.push(...children(next));
    }
    return false;
  };
  /** AE refuses to nest a comp inside itself or inside a comp it contains. */
  const canNest = (id: string, into = PM.proj?.compId) => !!into && exists(id) && !contains(id, into);

  const users = (id: string) => {
    const out: Array<{ comp: string; layer: any }> = [];
    for (const compId of allIds()) for (const layer of layersOf(compId)) {
      if (layer.type === 'precomp' && layer.d?.comp === id) out.push({ comp: compId, layer });
    }
    return out;
  };

  const list = () => allIds().map(id => {
    const comp = get(id);
    return {
      id, name: String(comp.name || 'Comp'), w: comp.w, h: comp.h, fps: comp.fps, dur: comp.dur, bg: comp.bg,
      open: isRoot(id), layers: (comp.layers || []).length, uses: users(id).length,
    };
  }).sort((a, b) => naturalCompare(a.name, b.name));

  const openTabs = (): string[] => {
    if (!PM.proj) return [];
    const ids = (tabs.get(PM.proj.id) || []).filter(exists);
    if (!ids.includes(PM.proj.compId)) ids.push(PM.proj.compId);
    tabs.set(PM.proj.id, ids);
    return ids;
  };

  const publishRotation = (before: any, after: any) => {
    const beforeId = before.compId, afterId = after.compId;
    const forward: Patch[] = [
      { path: ['comps', beforeId], exists: true, value: after.comps[beforeId] },
      { path: ['comps', afterId], exists: false },
      ...Object.keys(ROOT_TO_COMP).map(key => ({ path: [key], exists: Object.hasOwn(after, key), value: after[key] })),
    ];
    PM.bus.emit('history:project-patch', {
      projectId: after.id, revision: after.revision || 0, origin: 'interface',
      patches: JSON.parse(JSON.stringify(forward)), composition: true,
    });
  };

  /** Make `id` the open composition (the Timeline/viewer tab). */
  const open = (id: string, options: { tab?: boolean } = {}) => {
    if (!PM.proj || !exists(id)) return false;
    if (options.tab !== false && !openTabs().includes(id)) tabs.set(PM.proj.id, [...openTabs(), id]);
    if (isRoot(id)) { PM.bus.emit('comps'); return true; }
    if (PM.hist?.busy?.()) return false;
    PM.pause?.();
    const timeline = PM.Kernel?.services?.get?.('timeline');
    views.set(viewKey(PM.proj.compId), {
      time: PM.time, layers: [...PM.sel.layers],
      scrollT: timeline?.scrollT, scrollY: timeline?.scrollY, pps: timeline?.pps,
    });
    const before = PM.proj;
    const next = rotateProject(before, id);
    const view = views.get(viewKey(id));
    PM.replaceProject(next, { selection: { layers: view?.layers || [], keys: [], chan: null } });
    publishRotation(before, next);
    PM.time = PM.clamp(view?.time ?? 0, 0, next.dur);
    if (timeline) {
      if (Number.isFinite(view?.scrollT)) timeline.scrollT = view!.scrollT;
      if (Number.isFinite(view?.scrollY)) timeline.scrollY = view!.scrollY;
      if (Number.isFinite(view?.pps)) timeline.pps = view!.pps;
    }
    PM.bus.emit('time');
    PM.bus.emit('comps');
    PM.invalidate('all');
    return true;
  };

  /** Close a timeline tab. The last tab stays: the viewer always shows a comp. */
  const close = (id: string) => {
    const ids = openTabs();
    if (ids.length <= 1 || !ids.includes(id)) return false;
    const index = ids.indexOf(id);
    const rest = ids.filter(item => item !== id);
    tabs.set(PM.proj.id, rest);
    if (isRoot(id)) open(rest[Math.min(index, rest.length - 1)]!, { tab: false });
    PM.bus.emit('comps');
    return true;
  };

  const blankComp = (settings: any = {}) => {
    const base = PM.proj || {};
    const w = Math.round(settings.w ?? base.w ?? 1920), h = Math.round(settings.h ?? base.h ?? 1080);
    const fps = settings.fps ?? base.fps ?? 30, dur = settings.dur ?? base.dur ?? 10;
    const bg = settings.bg ?? '#000000';
    return {
      id: PM.uid('C'), name: String(settings.name || nextName('Comp')).trim(),
      w, h, fps, dur, bg, backgroundFill: PM.normalizeFill({ type: 'solid', color: bg }, bg),
      layers: [], comps: {}, markers: [], work: [0, dur], shutter: base.shutter ?? .5,
      ...(settings.render3d?{render3d:structuredClone(settings.render3d)}:{}),
    };
  };

  /** Composition ▸ New Composition. AE opens the new comp. */
  const create = (settings: any = {}, options: { open?: boolean; layers?: any[] } = {}) => {
    const comp: any = blankComp(settings);
    comp.name = uniqueName(comp.name);
    if (options.layers) comp.layers = options.layers;
    transact('New Composition', () => {
      PM.proj.comps = { ...comps(), [comp.id]: comp };
      PM.ProjectIndex?.invalidate();
    });
    PM.bus.emit('comps'); PM.bus.emit('project');
    if (options.open !== false) open(comp.id);
    return comp.id;
  };

  /** The live, mutable record for a comp (the open one maps onto PM.proj). */
  const writeSettings = (id: string, patch: any) => {
    const target: any = isRoot(id) ? PM.proj : comps()[id];
    if (!target) return;
    const nameKey = isRoot(id) ? 'compName' : 'name';
    if (patch.name != null && String(patch.name).trim()) {
      const previous = target[nameKey];
      target[nameKey] = uniqueName(String(patch.name).trim(), id);
      /* A precomp layer shows its source's name until it is renamed. */
      for (const { layer } of users(id)) if (layer.name === previous) layer.name = target[nameKey];
    }
    if (patch.w != null) target.w = Math.max(16, Math.round(Number(patch.w)));
    if (patch.h != null) target.h = Math.max(16, Math.round(Number(patch.h)));
    if (patch.fps != null) target.fps = PM.clamp(Number(patch.fps), 1, 240);
    if (patch.dur != null) {
      target.dur = Math.max(1 / (target.fps || 30), Number(patch.dur));
      const work = Array.isArray(target.work) ? target.work : [0, target.dur];
      target.work = [Math.min(work[0], target.dur), Math.min(Math.max(work[1], 0), target.dur)];
      if (target.work[1] <= target.work[0]) target.work = [0, target.dur];
    }
    if (patch.bg != null && /^#[0-9a-f]{6}$/i.test(patch.bg)) {
      target.bg = String(patch.bg).toUpperCase();
      target.backgroundFill = PM.normalizeFill({ type: 'solid', color: target.bg }, target.bg);
    }
    if (patch.w != null || patch.h != null) {
      for (const { layer } of users(id)) { layer.d.w = target.w; layer.d.h = target.h; }
    }
  };

  /** Composition ▸ Composition Settings. */
  const setSettings = (id: string, patch: any) => {
    if (!exists(id)) return false;
    transact('Composition Settings', () => writeSettings(id, patch));
    PM.touch?.();
    if (isRoot(id)) PM.time = PM.clamp(PM.time, 0, PM.proj.dur);
    PM.bus.emit('comps'); PM.bus.emit('project'); PM.bus.emit('layers');
    PM.invalidate('all');
    return true;
  };

  const rename = (id: string, name: string) => setSettings(id, { name });

  /** Fresh ids for layers copied into another composition, links kept. */
  const cloneLayers = (layers: any[]) => {
    const ids = new Map<string, string>();
    const copies = layers.map(layer => {
      const copy = PM.cloneLayer(layer);
      copy.name = layer.name;
      ids.set(layer.id, copy.id);
      return copy;
    });
    for (const copy of copies) {
      if (copy.parent) copy.parent = ids.get(copy.parent) ?? null;
      if (copy.group) copy.group = ids.get(copy.group) ?? null;
      if (copy.matteSource) copy.matteSource = ids.get(copy.matteSource) ?? null;
    }
    return copies;
  };

  /** Project panel ▸ Duplicate (⌘D). */
  const duplicate = (id: string) => {
    const source = get(id);
    if (!source) return null;
    const copy: any = JSON.parse(JSON.stringify(source));
    copy.id = PM.uid('C');
    copy.name = uniqueName(`${String(source.name).replace(/\s+\d+$/, '')} ${2}`);
    copy.layers = cloneLayers(source.layers || []);
    copy.comps = {};
    transact('Duplicate Composition', () => { PM.proj.comps = { ...comps(), [copy.id]: copy }; PM.ProjectIndex?.invalidate(); });
    PM.bus.emit('comps'); PM.bus.emit('project');
    return copy.id;
  };

  /** Project panel ▸ Delete. Layers that use the comp go with it, as in AE. */
  const remove = (id: string) => {
    if (!exists(id) || allIds().length <= 1) return false;
    if (isRoot(id)) {
      const fallback = openTabs().find(tab => tab !== id) || Object.keys(comps())[0]!;
      open(fallback, { tab: false });
    }
    const removed: string[] = [];
    transact('Delete Composition', () => {
      for (const compId of allIds()) {
        const target: any = isRoot(compId) ? PM.proj : comps()[compId];
        const doomed = new Set(target.layers.filter((layer: any) => layer.type === 'precomp' && layer.d?.comp === id).map((layer: any) => layer.id));
        if (!doomed.size) continue;
        removed.push(...doomed as Set<string>);
        target.layers = target.layers.filter((layer: any) => !doomed.has(layer.id));
        for (const layer of target.layers) if (doomed.has(layer.parent)) layer.parent = null;
      }
      const next = { ...comps() };
      delete next[id];
      PM.proj.comps = next;
      PM.ProjectIndex?.invalidate();
    });
    tabs.set(PM.proj.id, openTabs().filter(tab => tab !== id));
    PM.sel.layers = PM.sel.layers.filter((layerId: string) => !removed.includes(layerId));
    PM.bus.emit('comps'); PM.bus.emit('layers'); PM.bus.emit('sel'); PM.bus.emit('project');
    PM.invalidate('all');
    return true;
  };

  /** A precomp layer for `id`, placed the way AE places a comp dropped in a timeline. */
  const makeLayer = (id: string, host: any, from = 0) => {
    const comp = get(id);
    const layer = PM.mkLayer('precomp', { name: comp.name, d: { comp: id, w: comp.w, h: comp.h } }, host);
    layer.from = from;
    layer.dur = comp.dur;
    /* AE: anchor at the source's centre, positioned at the host comp's centre. */
    layer.p['anchor.x'].v = comp.w / 2; layer.p['anchor.y'].v = comp.h / 2;
    layer.p['position.x'].v = host.w / 2; layer.p['position.y'].v = host.h / 2;
    return layer;
  };

  /** Drag from the Project panel / “Add to timeline”: nest `id` in the open comp. */
  const addToTimeline = (id: string, options: { from?: number; index?: number; position?: number[] } = {}) => {
    if (!exists(id)) return null;
    if (!canNest(id)) {
      PM.toast?.(isRoot(id) ? 'A composition cannot contain itself' : `“${get(id).name}” already contains this composition`);
      return null;
    }
    const layer = makeLayer(id, PM.proj, options.from ?? PM.snapF(PM.time, PM.proj.fps));
    if (options.position?.every(Number.isFinite)) {
      layer.p['position.x'].v = options.position[0]; layer.p['position.y'].v = options.position[1];
    }
    transact('Add Composition', () => { PM.addLayer(layer, options.index ?? 0); PM.selectLayers(layer.id); });
    PM.invalidate('all');
    return layer;
  };

  /** Shift every time on a layer's timeline by `delta` seconds. */
  const shiftLayer = (layer: any, delta: number) => { layer.from = Math.max(0, layer.from + delta); };

  /**
   * Layer ▸ Pre-compose (⇧⌘C).
   * mode 'move': the selected layers move into a new comp that has the current
   *   comp's settings; one precomp layer takes their place.
   * mode 'leave': a single layer keeps its transforms, effects, masks and timing
   *   in this comp; only its source moves into a new comp the size of the source.
   */
  const precompose = (ids: string[], options: { name?: string; mode?: 'move' | 'leave'; adjustDuration?: boolean; open?: boolean } = {}) => {
    const host = PM.proj;
    const mode = options.mode || 'move';
    const selected = PM.expandGroups ? PM.expandGroups(ids) : ids;
    const picked = host.layers.filter((layer: any) => selected.includes(layer.id));
    if (!picked.length) return null;
    const name = uniqueName(String(options.name || nextName('Pre-comp')).trim());
    if (mode === 'leave') {
      const layer = picked[0];
      if (picked.length !== 1 || !canLeave(layer)) return null;
      return leaveAttributes(layer, name, options.open === true);
    }
    const pickedIds = new Set(picked.map((layer: any) => layer.id));
    let start = 0, dur = host.dur;
    if (options.adjustDuration) {
      start = Math.min(...picked.map((layer: any) => layer.from));
      dur = Math.max(1 / host.fps, Math.max(...picked.map((layer: any) => layer.from + layer.dur)) - start);
    }
    const comp: any = blankComp({ name, w: host.w, h: host.h, fps: host.fps, dur, bg: host.bg, render3d:host.render3d });
    comp.name = name;
    const index = host.layers.findIndex((layer: any) => pickedIds.has(layer.id));
    let layer: any = null;
    transact('Pre-compose', () => {
      comp.layers = picked;
      for (const moved of picked) {
        if (moved.parent && !pickedIds.has(moved.parent)) moved.parent = null;
        if (moved.group && !pickedIds.has(moved.group)) moved.group = null;
        if (moved.matteSource && !pickedIds.has(moved.matteSource)) moved.matteSource = null;
        if (start) shiftLayer(moved, -start);
      }
      PM.proj.comps = { ...comps(), [comp.id]: comp };
      layer = makeLayer(comp.id, host, start);
      layer.p['anchor.x'].v = comp.w / 2; layer.p['anchor.y'].v = comp.h / 2;
      const rest = host.layers.filter((item: any) => !pickedIds.has(item.id));
      for (const item of rest) {
        if (pickedIds.has(item.parent)) item.parent = null;
        if (pickedIds.has(item.matteSource)) item.matteSource = null;
      }
      rest.splice(Math.max(0, Math.min(index, rest.length)), 0, layer);
      host.layers = rest;
      PM.ProjectIndex?.invalidate();
      PM.selectLayers(layer.id);
    });
    PM.bus.emit('comps'); PM.bus.emit('layers'); PM.bus.emit('project');
    if (options.open) open(comp.id);
    PM.invalidate('all');
    return comp.id;
  };

  /* AE offers “Leave all attributes” only for a single layer with a source. */
  const LEAVE_TYPES = new Set(['solid', 'image', 'video', 'precomp', 'shader', 'extension']);
  const canLeave = (layer: any) => !!layer && LEAVE_TYPES.has(layer.type);

  const leaveAttributes = (layer: any, name: string, openAfter: boolean) => {
    const bounds = PM.GL?.bounds?.(layer, PM.time);
    const w = Math.max(16, Math.round(bounds?.w || layer.d?.w || PM.proj.w));
    const h = Math.max(16, Math.round(bounds?.h || layer.d?.h || PM.proj.h));
    /* Layer space for most sources starts at their centre; a precomp's starts
       at its top-left corner. Shift the anchor so nothing moves. */
    const ax = Number(bounds?.ax) || 0, ay = Number(bounds?.ay) || 0;
    /* The new comp spans the layer; the layer's source timing (trim, speed,
       time remap) moves inside with the source, so every frame is unchanged. */
    const comp: any = blankComp({ name, w, h, fps: PM.proj.fps, dur: layer.dur, bg: PM.proj.bg, render3d:PM.proj.render3d });
    comp.name = name;
    const inner = PM.mkLayer(layer.type, { name: layer.name, d: JSON.parse(JSON.stringify(layer.d)) }, comp);
    inner.from = 0;
    inner.dur = layer.dur;
    if (layer.type === 'video' || layer.type === 'image') { inner.p['position.x'].v = w / 2; inner.p['position.y'].v = h / 2; }
    transact('Pre-compose', () => {
      comp.layers = [inner];
      PM.proj.comps = { ...comps(), [comp.id]: comp };
      const shift = (prop: any, by: number) => {
        if (!prop) return;
        prop.v = Number(prop.v) + by;
        for (const key of prop.kf || []) key.v = Number(key.v) + by;
      };
      shift(layer.p['anchor.x'], ax); shift(layer.p['anchor.y'], ay);
      layer.type = 'precomp';
      layer.d = { comp: comp.id, w, h };
      PM.ProjectIndex?.invalidate();
    });
    PM.bus.emit('comps'); PM.bus.emit('layers'); PM.bus.emit('project');
    if (openAfter) open(comp.id);
    PM.invalidate('all');
    return comp.id;
  };

  /** Project panel ▸ New Comp from Selection: a comp matching the footage. */
  const fromAsset = (assetId: string) => {
    const meta: any = PM.proj?.assets?.[assetId];
    if (!meta) return null;
    const live = PM.assets?.get?.(assetId);
    const w = Math.round(meta.w || live?.w || PM.proj.w), h = Math.round(meta.h || live?.h || PM.proj.h);
    const dur = Number(meta.dur) > 0 ? Number(meta.dur) : PM.proj.dur;
    const name = uniqueName(String(meta.name || 'Comp').replace(/\.[a-z0-9]+$/i, ''));
    const id = create({ name, w, h, dur, fps: PM.proj.fps }, { open: true });
    if (PM.proj.compId === id) { PM.time = 0; PM.cmd?.('addFromAsset', assetId); }
    return id;
  };

  PM.Comps = {
    list, get, open, close, openTabs, create, setSettings, rename, duplicate, remove,
    addToTimeline, precompose, canLeave, canNest, contains, users, exists, fromAsset,
    isOpen: isRoot,
    active: () => PM.proj?.compId,
    normalize: (project: any) => normalizeCompositions(project, PM.uid),
    canonicalize: (forward: Patch[], backward: Patch[]) => canonicalizePatches(forward, backward, PM.proj?.compId),
    realize: (patches: Patch[]) => realizePatches(patches, PM.proj),
    /* Undoing a composition's creation while it is open: step out of it first. */
    prepareUndo: (patches: Patch[]) => {
      const rootId = PM.proj?.compId;
      if (!rootId || !patches.some(patch => patch.path.length === 2 && patch.path[0] === 'comps' && patch.path[1] === rootId && !patch.exists)) return;
      const doomed = new Set(patches.filter(patch => patch.path.length === 2 && patch.path[0] === 'comps' && !patch.exists).map(patch => patch.path[1]));
      const fallback = openTabs().find(id => !doomed.has(id)) || Object.keys(comps()).find(id => !doomed.has(id));
      if (fallback) open(fallback, { tab: false });
    },
  };
  PM.bus?.on?.('project', () => PM.bus.emit('comps'));
}
