/*
 * Caption files delivered beside an export: one per captions layer of the
 * open composition, in composition time from the start of the exported range.
 * A single layer writes "Film.srt"; several write "Film.en.srt",
 * "Film.fr.srt" (language when known, otherwise the layer name).
 */
import { exportCaptionsText } from './commands';
import type { CaptionFormat } from './formats';

export interface Sidecar { suffix: string | null; text: string; layerId: string }

/* Suffixes follow SIDECAR_SUFFIX_PATTERN, which main enforces: ASCII, a
   letter or digit first, at most 32 characters including a "-N" de-dup tail. */
const slug = (value: string) => value.normalize('NFKD').replace(/\p{M}+/gu, '')
  .replace(/[^A-Za-z0-9_-]+/g, '-').replace(/^[^A-Za-z0-9]+/, '').slice(0, 28).replace(/[-_]+$/, '') || 'captions';

export function captionSidecars(project: any, format: CaptionFormat, range: { from: number; to: number }): Sidecar[] {
  const layers = (project?.layers || []).filter((layer: any) => layer.type === 'captions' && layer.on !== false && layer.d?.cues?.length);
  const used = new Set<string>();
  return layers.map((layer: any) => {
    let suffix: string | null = null;
    if (layers.length > 1) {
      const base = slug(String(layer.d.language || layer.name || 'captions'));
      suffix = base;
      for (let n = 2; used.has(suffix.toLowerCase()); n++) suffix = `${base}-${n}`;
      used.add(suffix.toLowerCase());
    }
    return { suffix, layerId: layer.id, text: exportCaptionsText(layer, format, { from: range.from, to: range.to, rebase: range.from }) };
  });
}
