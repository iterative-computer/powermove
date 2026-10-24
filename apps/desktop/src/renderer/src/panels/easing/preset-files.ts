import { clampInfluence, linearKeyframeEase, segmentControlPoints, type TemporalEase } from '../../core/anim/temporal-ease';
import { cubic, normalize, validateCurve } from './curve';
import { DEFAULT_PREFERENCES, validateLibrary, type Group, type Library, type Preset } from './library';

/** Import data only. Components become independently editable native curves. */
export function importExternalPresets(value: unknown, uid: () => string): Preset[] {
  const source = value as Record<string, any>;
  if (!source || !Array.isArray(source.presetData) || source.presetData.length < 2 || source.presetData.length > 256) throw new Error('Invalid preset file');
  const first = source.presetData[0]?.value;
  const vector = Array.isArray(first), dimensions = vector ? first.length : 1;
  if (dimensions < 1 || dimensions > 4) throw new Error('Preset vectors must have 1–4 components');
  const interpolation = (code: number) => code === 6612 ? 'linear' as const : code === 6614 ? 'hold' as const : code === 6613 ? 'bezier' as const : (() => { throw new Error('Unknown preset interpolation'); })();
  const ease = (data: unknown, component: number): TemporalEase => {
    if (!Array.isArray(data) || (data.length !== 1 && data.length !== dimensions)) throw new Error('Preset handle dimensions do not match the values');
    const a = data[data.length === 1 ? 0 : component] as TemporalEase;
    if (!a || !Number.isFinite(a.speed) || !Number.isFinite(a.influence) || a.influence < .1 || a.influence > 100) throw new Error('Invalid preset handles');
    return { speed: a.speed, influence: clampInfluence(a.influence) };
  };
  // Validate the whole file first: an invalid component cannot partially import.
  for (const [i, key] of source.presetData.entries()) {
    if (!key || typeof key.time !== 'number' || !Number.isFinite(key.time)
      || (i && key.time <= source.presetData[i - 1].time)) throw new Error('Preset points must be in time order');
    const values = vector ? key.value : [key.value];
    if ((vector && !Array.isArray(key.value)) || (!vector && Array.isArray(key.value))
      || values.length !== dimensions || !values.every((v: unknown) => typeof v === 'number' && Number.isFinite(v))) throw new Error('Preset vectors must have matching finite numeric components');
    interpolation(key.intTypeIn); interpolation(key.intTypeOut);
    for (let axis = 0; axis < dimensions; axis++) { ease(key.tempEaseIn, axis); ease(key.tempEaseOut, axis); }
  }
  let color = typeof source.color === 'string' ? source.color : '';
  if (/^#[\da-f]{3}$/i.test(color)) color = '#' + color.slice(1).split('').map(x => x + x).join('');
  color = /^#[\da-f]{6}$/i.test(color) && !/^#(?:ffffff|000000)$/i.test(color) ? color : '';
  const name = typeof source.name === 'string' && source.name.trim() ? source.name.trim() : 'Imported easing';
  const labels = dimensions === 4 && /colou?r/i.test(name) ? ['R', 'G', 'B', 'A'] : ['X', 'Y', 'Z', 'W'];
  return Array.from({ length: dimensions }, (_, axis) => {
    const keys = source.presetData.map((key: any) => ({ ...linearKeyframeEase(), t: key.time, v: vector ? key.value[axis] : key.value,
      inInterp: interpolation(key.intTypeIn), outInterp: interpolation(key.intTypeOut),
      inEase: ease(key.tempEaseIn, axis), outEase: ease(key.tempEaseOut, axis) }));
    const flat = keys.every((k: any) => k.v === keys[0].v) && keys.every((k: any, i: number) => {
      if (i === keys.length - 1 || k.outInterp === 'hold' || keys[i + 1]!.inInterp === 'hold') return true;
      const segment = segmentControlPoints(k, keys[i + 1]!);
      return segment.p1.v === k.v && segment.p2.v === k.v;
    });
    // A constant dimension carries no easing. Keep it as a usable linear curve
    // rather than dropping the component (common for Z in 3D imports).
    const curve = flat ? cubic([1 / 3, 1 / 3, 2 / 3, 2 / 3]) : validateCurve(normalize(keys));
    return { id: uid(), name: vector ? `${name.slice(0, 95)} · ${labels[axis]}` : name.slice(0, 100), color, curve };
  });
}

/** Scalar convenience seam retained for callers that import one curve. */
export function importExternalPreset(value: unknown, uid: () => string): Preset {
  const presets = importExternalPresets(value, uid);
  if (presets.length !== 1) throw new Error('Use the component importer for vector presets');
  return presets[0]!;
}
export async function importPresetFiles(files: File[], uid: () => string): Promise<Library> {
  if (!files.length || files.length > 5000) throw new Error('Choose up to 5,000 preset files');
  if (files.reduce((n, f) => n + f.size, 0) > 10 * 1024 * 1024) throw new Error('Preset imports must be smaller than 10 MB');
  const groups: Group[] = [];
  const folderGroups = new Map<string, Group>();
  for (const file of files) {
    if (!/\.(json|kease)$/i.test(file.name)) continue;
    const data = JSON.parse(await file.text());
    if (/\.kease$/i.test(file.name) || data.presetData) {
      const name = file.webkitRelativePath?.split('/').slice(0, -1).join('/') || 'Imported';
      let group = folderGroups.get(name);
      if (!group) { group = { id: uid(), name: name.slice(-100), presets: [] }; groups.push(group); folderGroups.set(name, group); }
      group.presets.push(...importExternalPresets(data, uid));
    } else groups.push(...validateLibrary(data).groups.map(g => ({ ...g, id: uid(), presets: g.presets.map(p => ({ ...p, id: uid() })) })));
  }
  if (!groups.length) throw new Error('No supported preset files found. Choose JSON or .kease files.');
  return validateLibrary({ version: 1, active: groups[0]!.id, groups, preferences: { ...DEFAULT_PREFERENCES } });
}
