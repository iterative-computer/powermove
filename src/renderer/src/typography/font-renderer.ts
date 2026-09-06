import { inspectFont, isAxisTag } from './font-catalog';

const css = (value: string) => value.replace(/\\/g, '\\\\').replace(/"/g, '\\"');
export function variationEntries(content: any): Array<[string, number]> {
  return Object.entries(content ?? {}).flatMap(([key, value]) => {
    const tag = key.startsWith('fontAxis.') ? key.slice(9) : '';
    return isAxisTag(tag) && typeof value === 'number' && Number.isFinite(value) ? [[tag, value] as [string, number]] : [];
  }).sort(([a], [b]) => a.localeCompare(b));
}
export const variationSettings = (content: any) => variationEntries(content).map(([tag, value]) => `"${css(tag)}" ${value}`).join(', ');

/** Load the binary once per family. Binary FontFaces are synchronously loaded,
 * so a new animation sample never paints a temporary default-font frame.
 * Each sample still owns its face: simultaneous layers cannot change each other. */
export function createVariableFontRenderer(invalidate: () => void) {
  const sources = new Map<string, { data?: ArrayBuffer }>();
  const faces = new Map<string, FontFace>();
  const instance = Math.random().toString(36).slice(2);
  let sequence = 0;
  return (content: any): string | null => {
    const settings = variationSettings(content), family = String(content.font ?? '').trim();
    if (!settings || !family || typeof FontFace !== 'function') return null;
    let source = sources.get(family);
    if (!source) {
      source = {};
      sources.set(family, source);
      const pending = source;
      void (async () => {
        try {
          const font = await inspectFont(family);
          if (font.status !== 'variable' || !font.source) return;
          pending.data = await (await font.source.blob()).arrayBuffer();
          invalidate();
        } catch { sources.delete(family); }
      })();
    }
    if (!source.data) return null;
    const key = `${family}\n${settings}\n${!!content.italic}`;
    let face = faces.get(key);
    if (face) { faces.delete(key); faces.set(key, face); return face.family; }
    try {
      face = new FontFace(`Powermove Axis ${instance} ${++sequence}`, source.data, {
        weight: '1 1000', stretch: '1% 1000%', style: content.italic ? 'italic' : 'normal', variationSettings: settings
      } as FontFaceDescriptors & { variationSettings: string });
      if (face.status !== 'loaded') return null;
      document.fonts.add(face);
      faces.set(key, face);
      while (faces.size > 96) {
        const oldest = faces.keys().next().value!;
        document.fonts.delete(faces.get(oldest)!);
        faces.delete(oldest);
      }
      return face.family;
    } catch { return null; }
  };
}
