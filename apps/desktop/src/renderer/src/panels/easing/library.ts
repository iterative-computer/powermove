import { clone, cubic, point, validateCurve, type Curve } from './curve';
export interface Preset { id: string; name: string; color: string; curve: Curve }
export interface Group { id: string; name: string; presets: Preset[] }
export interface Preferences {
  view: 'both' | 'toggle'; tab: 'graph' | 'library'; columns: number; size: number;
  labels: 'none' | 'name' | 'influence'; palette: 'native' | 'blue' | 'green' | 'violet' | 'mono'; preview: boolean;
}
export interface Library { version: 1; groups: Group[]; active: string; preferences: Preferences }
export const STORAGE_KEY = 'pm.easing.library.v1';
export const DEFAULT_PREFERENCES: Preferences = { view: 'both', tab: 'graph', columns: 6, size: 44, labels: 'none', palette: 'native', preview: true };
export function defaults(): Library {
  const presets: Preset[] = [];
  const add = (name: string, curve: Curve) => presets.push({ id: `builtin-${presets.length}`, name, color: '', curve });
  for (const n of [.33, .5, .66, .8, .9]) add(`Ease ${Math.round(n * 100)}`, cubic([n, 0, 1 - n, 1]));
  add('Ease in', cubic([.7, 0, 1, 1])); add('Ease out', cubic([0, 0, .3, 1]));
  add('Soft', cubic([.2, 0, .2, 1])); add('Smooth', cubic([.6, 0, .1, 1]));
  add('Fast out', cubic([0, 0, .1, 1])); add('Fast in', cubic([.9, 0, 1, 1])); add('Linear', cubic([1 / 3, 1 / 3, 2 / 3, 2 / 3]));
  for (let i = 0; i < 6; i++) {
    const height = .08 + i * .035, reach = .45 + i * .035;
    const points = i === 4 ? [point(0, 0), point(.2, -.16), point(.65, 1.12), point(1, 1)] : [point(0, 0), point(reach, 1 + height), point(.78, 1 - height * .22), point(1, 1)];
    for (const p of points) { p.inInterp = p.outInterp = 'bezier'; p.inEase = { speed: 0, influence: 35 }; p.outEase = { speed: 0, influence: 35 }; }
    add(['Overshoot', 'Spring', 'Elastic', 'Pop', 'Anticipation', 'Soft spring'][i]!, { points });
  }
  for (let i = 0; i < 6; i++) {
    const h = .1 + i * .03;
    const points = [point(0, 0), point(.48, 1), point(.62, 1 - h), point(.76, 1), point(.85, 1 - h * .25), point(.93, 1), point(1, 1)];
    points.forEach((p, i) => { p.inInterp = p.outInterp = 'bezier'; p.inEase = { speed: i % 2 ? 3 : 0, influence: 33 }; p.outEase = { speed: i % 2 ? -3 : 0, influence: 33 }; });
    add(`Bounce ${i + 1}`, { points });
  }
  return { version: 1, groups: [{ id: 'default', name: '1', presets }], active: 'default', preferences: { ...DEFAULT_PREFERENCES } };
}
export function validateLibrary(value: unknown): Library {
  const source = value as Library;
  if (source?.version !== 1 || !Array.isArray(source.groups) || !source.groups.length || source.groups.length > 100) throw new Error('Choose an Easing library JSON file');
  const ids = new Set<string>();
  const text = (v: unknown, limit: number) => typeof v === 'string' && v.length > 0 && v.length <= limit;
  let total = 0;
  const groups = source.groups.map(g => {
    if (!text(g.id, 200) || ids.has(g.id) || !text(g.name, 100) || !Array.isArray(g.presets)) throw new Error('Invalid preset group');
    ids.add(g.id);
    const presets = g.presets.map(p => {
      if (++total > 5000 || !text(p.id, 200) || ids.has(p.id) || !text(p.name, 100)) throw new Error('Invalid preset');
      ids.add(p.id);
      return { id: p.id, name: p.name, color: /^#[\da-f]{6}$/i.test(p.color) ? p.color : '', curve: validateCurve(p.curve) };
    });
    return { id: g.id, name: g.name, presets };
  });
  const p = source.preferences || DEFAULT_PREFERENCES;
  return { version: 1, groups, active: groups.some(g => g.id === source.active) ? source.active : groups[0]!.id,
    preferences: { view: p.view === 'toggle' ? 'toggle' : 'both', tab: p.tab === 'library' ? 'library' : 'graph',
      columns: Number.isInteger(p.columns) && p.columns >= 0 && p.columns <= 12 ? p.columns : 6,
      size: [32, 44, 56, 72].includes(p.size) ? p.size : 44,
      labels: ['none', 'name', 'influence'].includes(p.labels) ? p.labels : 'none',
      palette: ['native', 'blue', 'green', 'violet', 'mono'].includes(p.palette) ? p.palette : 'native', preview: p.preview !== false } };
}
export function readLibrary(storage?: Pick<Storage, 'getItem'>): Library {
  try { const text = storage?.getItem(STORAGE_KEY); return text ? validateLibrary(JSON.parse(text)) : defaults(); } catch { return defaults(); }
}
export function mergeLibrary(current: Library, incoming: Library, uid: () => string): Library {
  const next = clone(current);
  for (const group of incoming.groups) next.groups.push({ ...clone(group), id: uid(), presets: group.presets.map(p => ({ ...clone(p), id: uid() })) });
  next.active = next.groups.at(-1)!.id;
  return validateLibrary(next);
}
export function movePreset(library: Library, id: string, groupId: string, beforeId?: string): Library {
  const next = clone(library), destination = next.groups.find(g => g.id === groupId);
  if (!destination) return next;
  const preset = next.groups.flatMap(g => g.presets).find(p => p.id === id);
  if (!preset || beforeId === id) return next;
  for (const g of next.groups) g.presets = g.presets.filter(p => p.id !== id);
  const at = beforeId ? destination.presets.findIndex(p => p.id === beforeId) : -1;
  destination.presets.splice(at < 0 ? destination.presets.length : at, 0, preset);
  return next;
}
