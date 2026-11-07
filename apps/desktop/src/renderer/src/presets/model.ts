/* My Presets: the view model behind Save Animation Preset… and the
   Effects & Presets browser. The engine (legacy/core/presets.ts, PM.Presets)
   owns capture, storage and apply; this file decides what to offer, how to
   describe it, and how to report the outcome. */

type PMLike = Record<string, any>;

export interface PresetEffectChoice {
  id: string;
  label: string;
  keys: number;
}

export interface PresetPropertyChoice {
  /** Stable row id: the pair name for Position/Scale/…, else the channel path. */
  id: string;
  label: string;
  group: string;
  paths: string[];
  keys: number;
}

export interface PresetChoices {
  effects: PresetEffectChoice[];
  properties: PresetPropertyChoice[];
}

export interface StoredPreset {
  id: string;
  name: string;
  effects: Array<{ type: string; parameters: Record<string, { kf?: unknown[] }>; enabled: unknown }>;
  properties: Array<{ path: string; channel: { kf?: unknown[] } }>;
}

/* The engine refuses these paths (project-local masks, shape groups, text
   animators, shader uniforms, transitions); never offer them. */
const NOT_PORTABLE = /^(?:m\.|g\.|mp\.|ta\.|ts\.|u\.|transition)/;

const keyCount = (prop: any): number => (Array.isArray(prop?.kf) ? prop.kf.length : 0);

function effectLabel(PM: PMLike, fx: any): string {
  return PM.effectDef?.(fx)?.label ?? PM.FX?.[fx.type]?.label ?? PM.Kernel?.effects?.get?.(fx.type)?.label ?? fx.type;
}

/** Everything on `layer` that a preset can carry, in inspector order. */
export function presetChoices(PM: PMLike, layer: any): PresetChoices {
  const fxIds = new Set<string>((layer?.fx ?? []).map((fx: any) => fx.id));
  const effects = (layer?.fx ?? [])
    .filter((fx: any) => !PM.effectDef || PM.effectDef(fx))
    .map((fx: any) => ({
      id: fx.id,
      label: effectLabel(PM, fx),
      keys: Object.values(fx.p ?? {}).reduce((sum: number, prop: any) => sum + keyCount(prop), 0) + keyCount(fx.on),
    }));
  /* Transform channels read as the inspector's rows: Position is one row
     covering x and y (and z on a 3D layer). */
  const pairs = new Map<string, string>();
  for (const [label, channels, prefix] of (PM.PAIRS ?? []) as Array<[string, string[], string]>) {
    for (const channel of channels) pairs.set(channel, label);
    if (prefix) pairs.set(prefix, label);
  }
  const rows = new Map<string, PresetPropertyChoice>();
  for (const { key, prop, label, group } of (PM.allProps?.(layer) ?? []) as Array<{ key: string; prop: any; label?: string; group?: string }>) {
    if (NOT_PORTABLE.test(key) || fxIds.has(key.split('.')[0]!) || key.endsWith('.$enabled')) continue;
    const transform = !!layer?.p?.[key];
    const pair = transform ? pairs.get(key) ?? pairs.get(key.split('.')[0]!) : undefined;
    const id = pair ?? key;
    const row = rows.get(id) ?? { id, label: pair ?? PM.CH?.[key]?.label ?? label ?? key, group: transform ? 'Transform' : group ?? 'Layer', paths: [] as string[], keys: 0 };
    row.paths.push(key);
    row.keys += keyCount(prop);
    rows.set(id, row);
  }
  return { effects, properties: [...rows.values()] };
}

export interface PresetSelection {
  effectIds: string[];
  propertyIds: string[];
}

/** What starts checked: what the user pointed at, else the layer's effects and animated properties. */
export function initialSelection(choices: PresetChoices, hint: { effectIds?: string[]; paths?: string[] } = {}): PresetSelection {
  const paths = hint.paths ?? [];
  if (hint.effectIds?.length || paths.length) {
    const effectIds = new Set<string>(hint.effectIds ?? []);
    for (const effect of choices.effects) if (paths.some((path) => path.startsWith(effect.id + '.'))) effectIds.add(effect.id);
    return {
      effectIds: choices.effects.filter((effect) => effectIds.has(effect.id)).map((effect) => effect.id),
      propertyIds: choices.properties.filter((row) => row.paths.some((path) => paths.includes(path))).map((row) => row.id),
    };
  }
  return {
    effectIds: choices.effects.map((effect) => effect.id),
    propertyIds: choices.properties.filter((row) => row.keys > 0).map((row) => row.id),
  };
}

const plural = (count: number, one: string, many = `${one}s`) => `${count} ${count === 1 ? one : many}`;

function listNames(names: string[]): string {
  if (names.length <= 1) return names[0] ?? '';
  if (names.length <= 3) return `${names.slice(0, -1).join(', ')} and ${names.at(-1)}`;
  return `${names.slice(0, 2).join(', ')} and ${names.length - 2} more`;
}

/** The sheet's one-line preview of what Save will write. */
export function selectionSummary(choices: PresetChoices, selection: PresetSelection, includeKeyframes: boolean): string {
  const effects = choices.effects.filter((effect) => selection.effectIds.includes(effect.id));
  const properties = choices.properties.filter((row) => selection.propertyIds.includes(row.id));
  if (!effects.length && !properties.length) return 'Choose at least one effect or property to save.';
  const names = [...effects.map((effect) => effect.label), ...properties.map((row) => row.label)];
  const keys = [...effects, ...properties].reduce((sum, item) => sum + item.keys, 0);
  if (includeKeyframes && keys) return `Saves ${listNames(names)} with ${plural(keys, 'keyframe')}, placed from the playhead when applied.`;
  return `Saves ${listNames(names)} as ${keys ? 'their values at the playhead, without keyframes' : 'they are now'}.`;
}

/** A short description for a saved preset's row: "Gaussian Blur, Opacity · animated". */
export function presetDetail(PM: PMLike, preset: StoredPreset): string {
  const names = [
    ...preset.effects.map((effect) => PM.FX?.[effect.type]?.label ?? PM.Kernel?.effects?.get?.(effect.type)?.label ?? effect.type),
    ...preset.properties.map((property) => PM.CH?.[property.path]?.label ?? property.path),
  ];
  const animated = preset.effects.some((effect) => Object.values(effect.parameters ?? {}).some((channel) => (channel?.kf?.length ?? 0) > 0)
    || ((effect.enabled as any)?.kf?.length ?? 0) > 0)
    || preset.properties.some((property) => (property.channel?.kf?.length ?? 0) > 0);
  return `${listNames([...new Set(names)])}${animated ? ' · animated' : ''}`;
}

export function uniquePresetName(PM: PMLike, base: string): string {
  const taken = new Set(((PM.Presets?.list?.() ?? []) as StoredPreset[]).map((preset) => preset.name));
  const trimmed = base.trim().slice(0, 74) || 'Preset';
  if (!taken.has(trimmed)) return trimmed;
  let n = 2;
  while (taken.has(`${trimmed} ${n}`)) n++;
  return `${trimmed} ${n}`;
}

/** Engine skip reasons ("Effect unavailable: tint") read as one gentle sentence. */
export function describeSkipped(skipped: string[]): string {
  if (!skipped.length) return '';
  const first = skipped[0]!.replace(/^([^:]+):\s*/, (_, reason: string) => `${reason.toLowerCase()} `).trim();
  return skipped.length === 1 ? `Skipped one item (${first}).` : `Skipped ${skipped.length} items that don’t fit this layer (${first}, …).`;
}

/** Apply a saved preset as one undo step, keyframes from the playhead. */
export function applyPreset(PM: PMLike, presetId: string, layerId?: string | null, time: number = PM.time): boolean {
  const preset = PM.Presets?.get?.(presetId) as StoredPreset | null;
  const layer = layerId ? PM.L?.(layerId) : PM.firstSel?.();
  if (!preset) { PM.toast?.('That preset is no longer available'); return false; }
  if (!layer) { PM.toast?.(`Select a layer to apply “${preset.name}”`); return false; }
  const result = PM.Edit.apply(
    { type: 'apply_preset', target: layer.id, preset: preset.id, time },
    { label: `Apply “${preset.name}”`, origin: 'fx-browser' },
  );
  if (result && result.ok === false) {
    PM.toast?.(String(result.message || `Couldn’t apply “${preset.name}”`).replace(/^No compatible effects or properties in preset\.?\s*/, `Nothing in “${preset.name}” fits ${layer.name}. `).trim(), 4200, { kind: 'alert' });
    return false;
  }
  const skipped: string[] = (result?.data?.results ?? []).flatMap((entry: any) => entry?.data?.skipped ?? []);
  PM.Inspector?.refresh?.();
  PM.invalidate?.();
  PM.toast?.(skipped.length
    ? `Applied “${preset.name}” to ${layer.name}. ${describeSkipped(skipped)}`
    : `Applied “${preset.name}” to ${layer.name}`, skipped.length ? 4200 : 2200, { error: false, key: 'preset-apply' });
  return true;
}
