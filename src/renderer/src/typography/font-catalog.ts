export interface FontAxis { tag: string; label: string; min: number; max: number; default: number }
export interface FontInspection {
  family: string;
  status: 'variable' | 'static' | 'missing' | 'unavailable';
  axes: FontAxis[];
  source?: { family: string; fullName?: string; postscriptName?: string; blob(): Promise<Blob> };
}
const labels: Record<string, string> = { wght: 'Weight', wdth: 'Width', opsz: 'Optical size', slnt: 'Slant', ital: 'Italic', GRAD: 'Grade' };
export const axisContentKey = (tag: string) => `fontAxis.${tag}`;
export const axisPath = (tag: string) => `c.${axisContentKey(tag)}`;
export const isAxisTag = (tag: string) => /^[\x20-\x7e]{4}$/.test(tag);

/** The file is the authority for axis names, limits and defaults, including custom axes. */
export function readFontAxes(buffer: ArrayBuffer): FontAxis[] {
  const v = new DataView(buffer), axes = new Map<string, FontAxis>();
  const within = (offset: number, length: number) => offset >= 0 && length >= 0 && offset + length <= v.byteLength;
  const tag = (offset: number) => String.fromCharCode(...new Uint8Array(buffer, offset, 4));
  if (!within(0, 12)) return [];
  const faces: number[] = [];
  if (tag(0) === 'ttcf') {
    const count = v.getUint32(8);
    if (count > 256 || !within(12, count * 4)) return [];
    for (let i = 0; i < count; i++) faces.push(v.getUint32(12 + i * 4));
  } else faces.push(0);
  for (const face of faces) {
    if (!within(face, 12)) continue;
    const count = v.getUint16(face + 4), tables = new Map<string, { start: number; end: number }>();
    if (!within(face + 12, count * 16)) continue;
    for (let i = 0; i < count; i++) {
      const record = face + 12 + i * 16, start = v.getUint32(record + 8), length = v.getUint32(record + 12);
      if (within(start, length)) tables.set(tag(record), { start, end: start + length });
    }
    const fvar = tables.get('fvar');
    if (!fvar || fvar.end - fvar.start < 16) continue;
    const names = new Map<number, string>(), nameTable = tables.get('name');
    if (nameTable && nameTable.end - nameTable.start >= 6) {
      const count = v.getUint16(nameTable.start + 2), strings = nameTable.start + v.getUint16(nameTable.start + 4);
      for (let i = 0; i < count; i++) {
        const r = nameTable.start + 6 + i * 12;
        if (r + 12 > nameTable.end) break;
        const platform = v.getUint16(r), language = v.getUint16(r + 4), id = v.getUint16(r + 6);
        const length = v.getUint16(r + 8), start = strings + v.getUint16(r + 10);
        if (start < nameTable.start || start + length > nameTable.end || (platform !== 0 && platform !== 3)) continue;
        let text = '';
        for (let n = start; n + 1 < start + length; n += 2) text += String.fromCharCode(v.getUint16(n));
        text = text.replace(/[\x00-\x1f\x7f]/g, '').trim();
        if (text && (!names.has(id) || language === 0x409)) names.set(id, text);
      }
    }
    const countAxes = v.getUint16(fvar.start + 8), stride = v.getUint16(fvar.start + 10);
    const startAxes = fvar.start + v.getUint16(fvar.start + 4);
    if (stride < 20 || countAxes > 128 || startAxes < fvar.start + 16) continue;
    for (let i = 0; i < countAxes; i++) {
      const r = startAxes + i * stride;
      if (r + 20 > fvar.end) break;
      const axis = tag(r), min = v.getInt32(r + 4) / 65536, def = v.getInt32(r + 8) / 65536, max = v.getInt32(r + 12) / 65536;
      if (!isAxisTag(axis) || min >= max || def < min || def > max || axes.has(axis)) continue;
      axes.set(axis, { tag: axis, label: names.get(v.getUint16(r + 18)) ?? labels[axis] ?? axis, min, max, default: def });
    }
  }
  return [...axes.values()];
}

const cache = new Map<string, Promise<FontInspection>>();
const normalize = (name: unknown) => String(name ?? '').trim().replace(/^['"]|['"]$/g, '').toLocaleLowerCase();
export function inspectFont(family: string, retry = false): Promise<FontInspection> {
  const key = normalize(family);
  if (retry) cache.delete(key);
  if (cache.has(key)) return cache.get(key)!;
  const pending = (async (): Promise<FontInspection> => {
    const empty = (status: FontInspection['status']): FontInspection => ({ family, status, axes: [] });
    if (!key) return empty('missing');
    if (typeof (globalThis as any).queryLocalFonts !== 'function') return empty('unavailable');
    try {
      const all = await (globalThis as any).queryLocalFonts();
      const candidates = (Array.isArray(all) ? all : []).filter((font: any) =>
        [font.family, font.fullName, font.postscriptName].some(name => normalize(name) === key));
      if (!candidates.length) return empty('missing');
      candidates.sort((a: any, b: any) => Number(/regular/i.test(b.style ?? b.fullName)) - Number(/regular/i.test(a.style ?? a.fullName)));
      let readable = 0;
      for (const source of candidates) {
        try {
          const axes = readFontAxes(await (await source.blob()).arrayBuffer());
          readable++;
          if (axes.length) return { family: source.family, status: 'variable', axes, source };
        } catch { /* A broken or duplicate face must not hide a readable variable face. */ }
      }
      return empty(readable === candidates.length ? 'static' : 'unavailable');
    } catch { return empty('unavailable'); }
  })();
  cache.set(key, pending);
  void pending.then(result => { if (result.status !== 'variable' && cache.get(key) === pending) cache.delete(key); });
  return pending;
}
