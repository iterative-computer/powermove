import { canAnimateContent, isProperty } from 'powermove';
import type { Channel, ChannelValue, ControlEditBinding, EditCommand, Layer, PowermoveAPI } from 'powermove';

export type InspectorAPI = {
  model: Pick<PowermoveAPI['model'], 'layer'>;
  selection: Pick<PowermoveAPI['selection'], 'layers' | 'keys'>;
  groups: Pick<PowermoveAPI['groups'], 'ancestors'>;
  transport: Pick<PowermoveAPI['transport'], 'time'>;
  anim: Pick<PowermoveAPI['anim'], 'findProp' | 'evP'>;
  edit: PowermoveAPI['edit'];
};

type InspectorLayer = Layer & {
  p: Record<string, Channel>;
  d: Record<string, any>;
  [key: string]: any;
};
const inspectable = (layer: Layer): InspectorLayer => layer as InspectorLayer;

type ContentPatch = Extract<EditCommand, { type: 'set_content' }>['patch'];

/** A selected group owns its selected members for shared inspector edits. */
export function inspectorSelection(api: InspectorAPI, layers: Layer[]): Layer[] {
  const ids = new Set(layers.map((layer) => layer.id));
  return layers.filter((layer) => !api.groups.ancestors(layer).some((group) => ids.has(group.id)));
}

export function translatePath(primary: Layer, target: Layer, path: string): string | null {
  if (path.startsWith('c.')) return canAnimateContent(target, path.slice(2)) ? path : null;
  if (path.startsWith('l.')) return target.type === 'audio' && path !== 'l.on' ? null : path;
  if (inspectable(target).p?.[path]) return path;
  if (path.startsWith('u.') || path.startsWith('x.')) return primary.type === target.type && (primary.type !== 'extension' || inspectable(primary).d.definition === inspectable(target).d.definition) ? path : null;
  if (/^(g|mp|ta|ts)\./.test(path)) return primary.id === target.id ? path : null;
  if (path.startsWith('m.')) {
    const [, id, ...rest] = path.split('.');
    const index = primary.masks.findIndex((mask) => mask.id === id);
    return target.masks?.[index] ? `m.${target.masks[index]!.id}.${rest.join('.')}` : null;
  }
  const [id, ...rest] = path.split('.');
  const index = primary.fx.findIndex((effect) => effect.id === id);
  return index >= 0 && target.fx?.[index]?.type === primary.fx[index]!.type
    ? `${target.fx[index]!.id}.${rest.join('.')}`
    : null;
}

/**
 * Content edits fan out to a different layer type only through the fields both
 * types declare as content channels — a fill set on a text layer also lands on
 * selected shapes and solids, while type-private data (an image's asset, a
 * shader's code) stays with its own kind.
 */
export function translateContentPatch(primary: Layer, target: Layer, patch: ContentPatch): ContentPatch | null {
  if (target.type === primary.type) return { ...patch };
  const shared = Object.entries(patch ?? {}).filter(([field]) => canAnimateContent(primary, field) && canAnimateContent(target, field));
  return shared.length ? Object.fromEntries(shared) : null;
}

function selectedLayers(api: InspectorAPI): Layer[] {
  return api.selection.layers().map((id) => api.model.layer(id)).filter((layer): layer is Layer => layer !== null);
}

export function evaluatedValue(api: InspectorAPI, layer: Layer, value: unknown, time: number, path: string): unknown {
  return isProperty(value) ? api.anim.evP(layer, value as Channel, time, path) : value;
}

function currentValue(api: InspectorAPI, layer: Layer | null, command: EditCommand): unknown {
  if (!layer) return undefined;
  if (command.type === 'set_content') {
    const field = Object.keys(command.patch ?? {})[0];
    return field ? evaluatedValue(api, layer, inspectable(layer).d[field], api.transport.time(), `c.${field}`) : undefined;
  }
  if (command.type === 'set_layer') {
    const field = Object.keys(command.patch ?? {})[0];
    const key = ({ visible: 'on', motionBlur: 'mblur' } as Record<string, string>)[field!] ?? field;
    return key ? evaluatedValue(api, layer, layer[key as keyof Layer], api.transport.time(), `l.${key}`) : undefined;
  }
  if (command.type === 'set_property') {
    const prop = api.anim.findProp(layer, command.path) ?? inspectable(layer).p?.[command.path];
    if (prop) return api.anim.evP(layer, prop, command.time ?? api.transport.time(), command.path);
    if (command.path.startsWith('c.')) return evaluatedValue(api, layer, inspectable(layer).d[command.path.slice(2)], api.transport.time(), command.path);
    if (command.path.startsWith('l.')) return evaluatedValue(api, layer, layer[command.path.slice(2) as keyof Layer], api.transport.time(), command.path);
  }
  return undefined;
}

export function inspectorTargets(api: InspectorAPI, layer: Layer, path: string) {
  const selected = inspectorSelection(api, selectedLayers(api));
  return (selected.some((candidate) => candidate.id === layer.id) ? selected : [layer])
    .filter((candidate) => !candidate.lock)
    .flatMap((candidate) => {
      const translated = translatePath(layer, candidate, path);
      return translated ? [{ layer: candidate, path: translated, prop: api.anim.findProp(candidate, translated) ?? inspectable(candidate).p?.[translated] }] : [];
    });
}

export function createInspectorEdit(api: InspectorAPI) {
  let snapshot: Map<string, unknown> | null = null;
  let editing = false;
  let keySnapshot = new Map<string, { base: number; keys: Array<{ t: number; v: number; [key: string]: unknown }> }>();

  const expandKeys = (commands: EditCommand[]): EditCommand[] => commands.flatMap((command) => {
    if (command.type !== 'set_property' || typeof command.value !== 'number' || command.mode === 'static') return [command];
    const layer = api.model.layer(String(command.target));
    if (!layer || layer.lock) return [command];
    const prop = api.anim.findProp(layer, command.path) ?? inspectable(layer).p?.[command.path];
    const selected = new Set(api.selection.keys());
    const keys = prop?.kf?.filter((key) => selected.has(key.i) && typeof key.v === 'number') ?? [];
    if (keys.length < 2) return [command];
    const id = `${layer.id}:${command.path}`;
    if (!keySnapshot.has(id)) keySnapshot.set(id, {
      base: Number(currentValue(api, layer, command)),
      keys: keys.map((key) => ({ ...key, v: Number(key.v) }))
    });
    const initial = keySnapshot.get(id)!;
    const delta = command.value;
    return initial.keys.map((key) => ({
      ...command,
      mode: 'keyframe',
      time: layer.from + key.t,
      value: key.v + delta - initial.base
    }));
  });

  const expand = (input: EditCommand | EditCommand[], relative = false): EditCommand[] => {
    const allSelected = selectedLayers(api);
    const selected = inspectorSelection(api, allSelected).filter((layer) => !layer.lock);
    const first = inspectorSelection(api, allSelected)[0];
    return ([] as EditCommand[]).concat(input).flatMap((command) => {
      if (!first || !('target' in command) || command.target !== first.id || allSelected.length < 2) return [command];
      if (!['set_property', 'set_expression', 'replace_keyframes', 'set_content', 'set_layer'].includes(command.type)) return [command];
      return selected.flatMap((target) => {
        const sourcePath = 'path' in command ? command.path : null;
        const path = sourcePath ? translatePath(first, target, sourcePath) : null;
        if (sourcePath && !path) return [];
        const patch = command.type === 'set_content' ? translateContentPatch(first, target, command.patch) : null;
        if (command.type === 'set_content' && !patch) return [];
        const next = { ...command, target: target.id, ...(path ? { path } : {}), ...(patch ? { patch } : {}) } as EditCommand;
        if (relative && command.type === 'set_property' && typeof command.value === 'number' && path && next.type === 'set_property') {
          const key = `${target.id}:${path}`;
          const firstKey = `${first.id}:${sourcePath}`;
          if (!snapshot!.has(key)) snapshot!.set(key, currentValue(api, target, next));
          if (!snapshot!.has(firstKey)) snapshot!.set(firstKey, currentValue(api, first, command));
          const own = snapshot!.get(key);
          const base = snapshot!.get(firstKey);
          if (typeof own === 'number' && typeof base === 'number') next.value = own + command.value - base;
        }
        if (relative && command.type === 'set_content' && next.type === 'set_content') {
          for (const [field, value] of Object.entries(next.patch)) if (typeof value === 'number') {
            const key = `${target.id}:c.${field}`;
            const firstKey = `${first.id}:c.${field}`;
            const read = (candidate: Layer) => currentValue(api, candidate, { type: 'set_content', target: candidate.id, patch: { [field]: value } });
            if (!snapshot!.has(key)) snapshot!.set(key, read(target));
            if (!snapshot!.has(firstKey)) snapshot!.set(firstKey, read(first));
            const own = snapshot!.get(key);
            const base = snapshot!.get(firstKey);
            if (typeof own === 'number' && typeof base === 'number') next.patch[field] = own + value - base;
          }
        }
        return [next];
      });
    });
  };

  return {
    begin(label: string, meta?: Parameters<PowermoveAPI['edit']['begin']>[1]) {
      editing = true;
      snapshot = new Map();
      keySnapshot = new Map();
      return api.edit.begin(label, meta);
    },
    dispatch(commands: EditCommand | EditCommand[]) {
      const next = expandKeys(expand(commands, editing));
      let result = { ok: true } as ReturnType<PowermoveAPI['edit']['dispatch']>;
      for (const command of next) result = api.edit.dispatch(command);
      return result;
    },
    apply(commands: EditCommand | EditCommand[], meta?: Parameters<PowermoveAPI['edit']['apply']>[1]) {
      keySnapshot = new Map();
      const next = expandKeys(expand(commands));
      const input = Array.isArray(commands) || next.length !== 1 ? next : next[0]!;
      return meta === undefined ? api.edit.apply(input) : api.edit.apply(input, meta);
    },
    commit(label?: string) {
      editing = false;
      snapshot = null;
      keySnapshot = new Map();
      return api.edit.commit(label);
    },
    cancel() {
      editing = false;
      snapshot = null;
      keySnapshot = new Map();
      return api.edit.cancel();
    },
    mutate: api.edit.mutate
  };
}

export function inspectorMixed(api: InspectorAPI, binding: ControlEditBinding, value: unknown): boolean {
  if (binding.mode !== 'command' || typeof binding.command !== 'function') return false;
  const selected = inspectorSelection(api, selectedLayers(api)).filter((layer) => !layer.lock);
  if (selected.length < 2) return false;
  const primary = selected[0]!;
  const commands = ([] as EditCommand[]).concat(binding.command(value));
  const values = commands.flatMap((command) => selected.flatMap((target) => {
    if (!('target' in command) || command.target !== primary.id) return [];
    const sourcePath = 'path' in command ? command.path : null;
    const path = sourcePath ? translatePath(primary, target, sourcePath) : null;
    if (sourcePath && !path) return [];
    const patch = command.type === 'set_content' ? translateContentPatch(primary, target, command.patch) : null;
    if (command.type === 'set_content' && !patch) return [];
    const next = { ...command, target: target.id, ...(path ? { path } : {}), ...(patch ? { patch } : {}) } as EditCommand;
    return ['set_property', 'set_content', 'set_layer'].includes(next.type) ? [currentValue(api, target, next)] : [];
  }));
  return values.length > 1 && values.some((candidate) => !Object.is(candidate, values[0]));
}

interface LegacyInspectorSource {
  time: number;
  sel: { layers: string[]; keys: string[]; chan?: string | null };
  Edit: PowermoveAPI['edit'];
  L(id: string): Layer | null;
  selLayers(): Layer[];
  groupAncestors?(layer: Layer): Layer[];
  findProp?(layer: Layer, path: string): Channel | null;
  evP(layer: Layer, property: Channel, time: number, path: string): ChannelValue | null;
}

function isLegacyInspectorSource(value: unknown): value is LegacyInspectorSource {
  if (!value || typeof value !== 'object') return false;
  const source = value as Partial<LegacyInspectorSource>;
  return typeof source.time === 'number'
    && !!source.sel
    && !!source.Edit
    && typeof source.L === 'function'
    && typeof source.selLayers === 'function'
    && typeof source.evP === 'function';
}

/** @deprecated Compatibility for one renderer behavior suite pending its Phase 4 test-fixture move. */
export function inspectorPM(value: unknown) {
  if (!isLegacyInspectorSource(value)) throw new TypeError('Invalid legacy inspector test source');
  const source = value;
  const base: InspectorAPI = {
    model: { layer: (id: string) => source.L(id) },
    selection: {
      layers: () => source.selLayers().map((layer) => layer.id),
      keys: () => source.sel.keys
    },
    groups: { ancestors: (layer: Layer) => source.groupAncestors?.(layer) ?? [] },
    transport: { time: () => source.time },
    anim: {
      findProp: (layer, path) => source.findProp?.(layer, path) ?? null,
      evP: source.evP.bind(source)
    },
    edit: source.Edit
  };
  const edit = createInspectorEdit(base);
  return {
    ...base,
    Edit: edit,
    inspectorMixed: (binding: unknown, nextValue: unknown) => {
      if (!binding || typeof binding !== 'object') return false;
      const candidate = binding as { mode?: unknown; command?: unknown };
      if (candidate.mode !== 'command' || typeof candidate.command !== 'function') return false;
      const normalized: ControlEditBinding = {
        mode: 'command',
        label: 'Inspector edit',
        command: candidate.command as (input: unknown) => EditCommand | EditCommand[]
      };
      return inspectorMixed(base, normalized, nextValue);
    }
  };
}
