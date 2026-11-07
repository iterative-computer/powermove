import type { PMRegistry } from '../registry';
import type { Keyframe, Channel } from '../../core/types/project';
import type { EffectInput } from '../../core/types/project';

export interface PresetChannel {
  v: any;
  /** Seconds relative to the capture playhead, with source key IDs removed. */
  kf: Omit<Keyframe, 'i'>[];
}
export interface AnimationPreset {
  schemaVersion: 1;
  id: string;
  name: string;
  effects: { type: string; parameters: Record<string, PresetChannel>; enabled: boolean | PresetChannel; inputs?: Record<string, EffectInput> }[];
  properties: { path: string; channel: PresetChannel }[];
}
export interface CapturePresetOptions {
  name: string;
  layerId?: string;
  /** Omit to capture the whole stack; [] captures properties only. Stack order is preserved. */
  effectIds?: string[];
  /** Portable channel paths (e.g. position.x, rotation, c.size, x.amount). */
  propertyPaths?: string[];
  includeAnimation?: boolean;
  time?: number;
}
const KEY = 'animationPresets';
const clone = <T>(value: T): T => JSON.parse(JSON.stringify(value));
const validName = (name: string) => {
  if (typeof name !== 'string' || !name.trim() || name.trim().length > 80) throw new Error('Name the preset (up to 80 characters).');
  return name.trim();
};
const portable = (path: string) => !path.split('.').some(part => ['__proto__', 'prototype', 'constructor'].includes(part))
  && !/^(?:m\.|g\.|mp\.|ta\.|ts\.|u\.|transition)/.test(path);

/** User presets use the same durable PM.store service as render presets and workspaces. */
export function install(PM: PMRegistry): void {
  const list = (): AnimationPreset[] => {
    const saved = PM.store.get(KEY, []);
    return Array.isArray(saved) ? clone(saved.filter(p => p?.schemaVersion === 1 && typeof p.id === 'string'
      && typeof p.name === 'string' && Array.isArray(p.effects) && Array.isArray(p.properties))) : [];
  };
  const write = (presets: AnimationPreset[]) => { PM.store.set(KEY, clone(presets)); PM.bus.emit('presets'); };
  const layerFor = (id?: string) => {
    const layer = id ? PM.L(id) : PM.firstSel();
    if (!layer) throw new Error('Select a layer first.');
    return layer;
  };
  const atTime = (time?: number) => {
    const at = time ?? PM.time;
    if (typeof at !== 'number' || !Number.isFinite(at)) throw new Error('Time must be finite.');
    return at;
  };
  const capture = (options: CapturePresetOptions): AnimationPreset => {
    const name = validName(options.name), layer = layerFor(options.layerId), time = atTime(options.time);
    const channel = (prop: Channel<any>, path: string): PresetChannel => ({
      v: clone(PM.evP(layer, prop, time, path)),
      kf: options.includeAnimation ? clone(prop.kf || []).map((key: any) => {
        delete key.i; key.t += layer.from - time; return key;
      }) : [],
    });
    const requested = options.effectIds && new Set(options.effectIds);
    if (requested && [...requested].some(id => !layer.fx.some((fx: any) => fx.id === id))) throw new Error('Selected effect not found.');
    const effects = (layer.fx || []).filter((fx: any) => !requested || requested.has(fx.id)).map((fx: any) => {
      if (!PM.effectDef(fx)) throw new Error(`Cannot save unavailable effect: ${fx.type}`);
      return { type: fx.type, parameters: Object.fromEntries(Object.entries(fx.p).map(([key, prop]) => [key, channel(prop as Channel<any>, `${fx.id}.${key}`)])),
        enabled: typeof fx.on === 'boolean' ? fx.on : channel(fx.on, `${fx.id}.$enabled`),
        ...(fx.inputs ? {inputs: clone(fx.inputs)} : {}) };
    });
    const properties = [...new Set(options.propertyPaths || [])].map(path => {
      const prop = PM.findProp(layer, path);
      if (!portable(path) || (layer.fx || []).some((fx: any) => path.startsWith(fx.id + '.')) || !prop) throw new Error(`Choose a portable layer property: ${path}`);
      return {path, channel: channel(prop, path)};
    });
    if (!effects.length && !properties.length) throw new Error('Select effects or properties to save.');
    return {schemaVersion: 1, id: PM.uid('preset'), name, effects, properties};
  };
  // Runs inside Edit's lock checks, rollback, history, graph validation and notifications.
  const applyToLayer = (id: string, layer: any, time?: number) => {
    const preset = list().find(p => p.id === id);
    if (!preset) throw new Error('Preset not found.');
    const at = atTime(time), offset = at - layer.from, skipped: string[] = [], effectIds: string[] = [];
    let appliedProperties = 0;
    const restore = (saved: PresetChannel, base: any) => {
      const sourceKeys = saved.kf?.length ? saved.kf : base.kf?.length ? [PM.KF(0, saved.v)] : [];
      const keys = sourceKeys.map((key: Omit<Keyframe, 'i'>) => ({ ...clone(key), i: PM.uid('k'), t: key.t + offset }));
      const existing = (base.kf || []).filter((key: any) => !keys.some((k: Keyframe) => Math.abs(k.t - key.t) <= 1e-6));
      return { ...base, v: clone(saved.v), expr: null,
        kf: PM.normalizeKeyframes([...existing, ...keys], saved.v, PM.proj.fps, -Infinity) };
    };
    for (const saved of preset.effects) {
      // Resolve the current definition so newly introduced parameters keep their defaults.
      const fx = PM.mkEffect(saved.type, undefined, layer);
      if (!fx) { skipped.push(`Effect unavailable: ${saved.type}`); continue; }
      if (PM.TYPE_META[layer.type]?.effects === false) { skipped.push(`Effects unsupported on ${layer.type}`); continue; }
      for (const [key, value] of Object.entries(saved.parameters)) {
        if (!Object.hasOwn(fx.p, key)) { skipped.push(`Parameter unavailable: ${saved.type}.${key}`); continue; }
        if (typeof fx.p[key].v !== typeof value.v) { skipped.push(`Parameter type changed: ${saved.type}.${key}`); continue; }
        fx.p[key] = restore(value, fx.p[key]);
      }
      fx.on = typeof saved.enabled === 'boolean' ? saved.enabled : restore(saved.enabled, PM.P(true));
      // References are project-local. Retain available inputs and leave missing
      // ones at the definition's empty default instead of creating broken links.
      for (const [key, input] of Object.entries(saved.inputs || {})) {
        if (!Object.hasOwn(fx.inputs || {}, key)) { skipped.push(`Input unavailable: ${saved.type}.${key}`); continue; }
        if (input.layerId && !PM.L(input.layerId) || input.assetId && !PM.assets.get(input.assetId)) {
          skipped.push(`Input source unavailable: ${saved.type}.${key}`); continue;
        }
        fx.inputs[key] = clone(input);
      }
      layer.fx.push(fx); effectIds.push(fx.id);
    }
    for (const {path, channel} of preset.properties) {
      const prop = portable(path) && PM.findProp(layer, path);
      if (!prop || typeof prop.v !== typeof channel.v) { skipped.push(`Property unavailable: ${path}`); continue; }
      if (layer.locked_intent?.[path] || layer.locked_intent?.[path.split('.')[0]!]) throw new Error(`Preserved hand-edited ${path}`);
      Object.assign(prop, restore(channel, prop)); appliedProperties++;
    }
    if (!effectIds.length && !appliedProperties) throw new Error(`No compatible effects or properties in preset. ${skipped.join('. ')}`);
    return {presetId: id, effectIds, appliedProperties, skipped};
  };
  PM.Presets = {
    list, capture,
    get: (id: string) => list().find(p => p.id === id) || null,
    save(options: CapturePresetOptions) { const preset = capture(options); write([...list(), preset]); return clone(preset); },
    // expectedName lets Undo avoid overwriting a later rename from another window.
    rename(id: string, name: string, expectedName?: string) {
      const next = list(), preset = next.find(p => p.id === id); if (!preset) return false;
      if (expectedName !== undefined && preset.name !== expectedName) return false;
      preset.name = validName(name); write(next); return true;
    },
    remove(id: string) { return PM.Presets.removeWithUndo(id) !== null; },
    /** Keep this detached receipt in the delete toast; remove() remains boolean-compatible. */
    removeWithUndo(id: string): {preset: AnimationPreset; index: number} | null {
      const current = list(), index = current.findIndex(p => p.id === id);
      if (index < 0) return null;
      const [preset] = current.splice(index, 1); write(current);
      return {preset: clone(preset!), index};
    },
    /** Read the latest library, preserving other windows' additions and refusing ID collisions. */
    restore(preset: AnimationPreset, index: number): boolean {
      if (!preset || preset.schemaVersion !== 1 || typeof preset.id !== 'string' || !preset.id
          || !Array.isArray(preset.effects) || !Array.isArray(preset.properties)
          || !Number.isInteger(index) || index < 0) return false;
      const restored = clone(preset); restored.name = validName(restored.name);
      const current = list();
      if (current.some(p => p.id === restored.id)) return false;
      current.splice(Math.min(index, current.length), 0, restored); write(current); return true;
    },
    apply(id: string, layerId?: string, time?: number) {
      return PM.Edit.apply({type: 'apply_preset', preset: id, target: layerId, time}, {label: 'Apply preset', origin: 'effects-panel'});
    },
    applyToLayer,
  };
  PM.bus.on('store:external', (keys: string[]) => { if (keys.includes(KEY)) PM.bus.emit('presets'); });
}
