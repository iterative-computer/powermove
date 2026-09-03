export interface FontVariationAxis {
  tag: string;
  label: string;
  min: number;
  max: number;
  default: number;
}

export const FONT_AXIS_PREFIX = 'fontAxis.';

const COMMON_AXES: Record<string, Omit<FontVariationAxis, 'tag'>> = {
  wght: { label: 'Weight', min: 1, max: 1000, default: 400 },
  wdth: { label: 'Width', min: 25, max: 200, default: 100 },
  slnt: { label: 'Slant', min: -90, max: 90, default: 0 },
  ital: { label: 'Italic', min: 0, max: 1, default: 0 },
  opsz: { label: 'Optical size', min: 6, max: 144, default: 14 },
  GRAD: { label: 'Grade', min: -200, max: 200, default: 0 },
};

const discoveryCache = new Map<string, Promise<FontVariationAxis[]>>();

export function validAxisTag(value: unknown): value is string {
  return typeof value === 'string' && /^[\x20-\x7e]{4}$/.test(value);
}

export function normalizeAxisTag(value: unknown): string | null {
  const raw = String(value ?? '').trim();
  if (!validAxisTag(raw)) return null;
  const common = raw.toLowerCase();
  return common in COMMON_AXES ? common : raw;
}

export function fontAxisContentKey(tag: string): string {
  return FONT_AXIS_PREFIX + tag;
}

export function fontAxisChannelPath(tag: string): string {
  return 'c.' + fontAxisContentKey(tag);
}

export function axisMeta(tag: string, fallback = 0): FontVariationAxis {
  const common = COMMON_AXES[tag];
  return common ? { tag, ...common } : {
    tag,
    label: tag,
    min: Math.min(-100, fallback),
    max: Math.max(100, fallback),
    default: fallback,
  };
}

function fixed(view: DataView, offset: number): number {
  return view.getInt32(offset, false) / 65536;
}

function tag(view: DataView, offset: number): string {
  return String.fromCharCode(
    view.getUint8(offset), view.getUint8(offset + 1),
    view.getUint8(offset + 2), view.getUint8(offset + 3)
  );
}

function sfntOffset(view: DataView): number | null {
  if (view.byteLength < 12) return null;
  if (tag(view, 0) !== 'ttcf') return 0;
  const count = view.getUint32(8, false);
  return count > 0 && view.byteLength >= 16 ? view.getUint32(12, false) : null;
}

function tables(view: DataView, offset: number): Map<string, { offset: number; length: number }> {
  const output = new Map<string, { offset: number; length: number }>();
  if (offset < 0 || offset + 12 > view.byteLength) return output;
  const count = view.getUint16(offset + 4, false);
  for (let index = 0; index < count; index += 1) {
    const record = offset + 12 + index * 16;
    if (record + 16 > view.byteLength) break;
    const tableOffset = view.getUint32(record + 8, false);
    const length = view.getUint32(record + 12, false);
    if (tableOffset <= view.byteLength && length <= view.byteLength - tableOffset) {
      output.set(tag(view, record), { offset: tableOffset, length });
    }
  }
  return output;
}

function names(view: DataView, table?: { offset: number; length: number }): Map<number, string> {
  const output = new Map<number, string>();
  if (!table || table.length < 6) return output;
  const end = table.offset + table.length;
  const count = view.getUint16(table.offset + 2, false);
  const strings = table.offset + view.getUint16(table.offset + 4, false);
  const candidates = new Map<number, Array<{ score: number; value: string }>>();
  for (let index = 0; index < count; index += 1) {
    const record = table.offset + 6 + index * 12;
    if (record + 12 > end) break;
    const platform = view.getUint16(record, false);
    const language = view.getUint16(record + 4, false);
    const nameId = view.getUint16(record + 6, false);
    const length = view.getUint16(record + 8, false);
    const offset = strings + view.getUint16(record + 10, false);
    if (offset < table.offset || offset + length > end) continue;
    let value = '';
    if (platform === 0 || platform === 3) {
      for (let cursor = offset; cursor + 1 < offset + length; cursor += 2) {
        value += String.fromCharCode(view.getUint16(cursor, false));
      }
    } else if (platform === 1) {
      for (let cursor = offset; cursor < offset + length; cursor += 1) value += String.fromCharCode(view.getUint8(cursor));
    } else continue;
    value = value.replace(/[\u0000-\u001f\u007f]/g, '').trim();
    if (!value) continue;
    const score = platform === 3 && language === 0x0409 ? 3 : platform === 0 ? 2 : 1;
    const list = candidates.get(nameId) ?? [];
    list.push({ score, value });
    candidates.set(nameId, list);
  }
  for (const [nameId, values] of candidates) {
    values.sort((a, b) => b.score - a.score);
    output.set(nameId, values[0]!.value);
  }
  return output;
}

/** Read the OpenType `fvar` table without bringing font binaries into project
    state. System-font access is used only for inspector metadata. */
export function parseOpenTypeVariationAxes(buffer: ArrayBuffer): FontVariationAxis[] {
  const view = new DataView(buffer);
  const offset = sfntOffset(view);
  if (offset == null) return [];
  const directory = tables(view, offset);
  const fvar = directory.get('fvar');
  if (!fvar || fvar.length < 16) return [];
  const labels = names(view, directory.get('name'));
  const axesOffset = fvar.offset + view.getUint16(fvar.offset + 4, false);
  const axisCount = view.getUint16(fvar.offset + 8, false);
  const axisSize = view.getUint16(fvar.offset + 10, false);
  if (axisSize < 20 || axisCount > 128) return [];
  const output: FontVariationAxis[] = [];
  for (let index = 0; index < axisCount; index += 1) {
    const record = axesOffset + index * axisSize;
    if (record < fvar.offset || record + 20 > fvar.offset + fvar.length) break;
    const axisTag = tag(view, record);
    if (!validAxisTag(axisTag)) continue;
    const min = fixed(view, record + 4);
    const defaultValue = fixed(view, record + 8);
    const max = fixed(view, record + 12);
    if (![min, defaultValue, max].every(Number.isFinite) || min > max) continue;
    const nameId = view.getUint16(record + 18, false);
    output.push({
      tag: axisTag,
      label: labels.get(nameId) || COMMON_AXES[axisTag]?.label || axisTag,
      min,
      max,
      default: Math.max(min, Math.min(max, defaultValue)),
    });
  }
  return output;
}

export async function discoverVariableFontAxes(family: string): Promise<FontVariationAxis[]> {
  const cleanFamily = String(family || '').trim();
  if (!cleanFamily || typeof (window as any).queryLocalFonts !== 'function') return [];
  let cached = discoveryCache.get(cleanFamily);
  if (cached) return cached;
  cached = (async () => {
    try {
      const fonts = await (window as any).queryLocalFonts();
      const matches = (Array.isArray(fonts) ? fonts : []).filter((font: any) =>
        String(font?.family || '').trim().toLocaleLowerCase() === cleanFamily.toLocaleLowerCase()
        && typeof font?.blob === 'function');
      const merged = new Map<string, FontVariationAxis>();
      for (const font of matches) {
        const axes = parseOpenTypeVariationAxes(await (await font.blob()).arrayBuffer());
        for (const axis of axes) if (!merged.has(axis.tag)) merged.set(axis.tag, axis);
      }
      return [...merged.values()];
    } catch {
      return [];
    }
  })();
  discoveryCache.set(cleanFamily, cached);
  return cached;
}

export function persistedFontAxes(content: Record<string, any>, detected: FontVariationAxis[] = []): FontVariationAxis[] {
  const byTag = new Map(detected.map((axis) => [axis.tag, axis]));
  return Object.entries(content ?? {}).flatMap(([key, property]) => {
    if (!key.startsWith(FONT_AXIS_PREFIX) || !property || typeof property !== 'object' || !Array.isArray(property.kf)) return [];
    const axisTag = key.slice(FONT_AXIS_PREFIX.length);
    if (!validAxisTag(axisTag)) return [];
    return [byTag.get(axisTag) ?? axisMeta(axisTag, Number(property.v) || 0)];
  }).sort((a, b) => a.label.localeCompare(b.label));
}
