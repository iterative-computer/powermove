/*
 * Execution of the caption edit commands (add_captions, edit_captions).
 * Called inside the guarded Edit transaction in legacy/core/editing.ts, so
 * history, provenance, lock policy and rollback are the same as every other
 * command. Times in commands are composition seconds; layers store cues in
 * layer time.
 */
import { formatCaptions, parseCaptions, type CaptionFormat } from './formats';
import {
  captionsEnd, cueId, deleteCues, insertCues, mergeCues, moveCues, normalizeCaptionsContent, normalizeCues, patchStyle,
  presetStyle, splitCue, updateCues, type CaptionCue, type CaptionStyle, type CuePatch
} from './model';

export const CAPTION_EDIT_OPS = ['replace', 'insert', 'update', 'delete', 'split', 'merge', 'move', 'import', 'style'] as const;
export type CaptionEditOp = (typeof CAPTION_EDIT_OPS)[number];

interface Context {
  PM: any;
  findLayer(ref: unknown): any;
}

type CueInput = { id?: unknown; start?: unknown; end?: unknown; text?: unknown; words?: unknown };

const toLayerTime = (cues: readonly CueInput[] | undefined, shift: number) => (cues ?? []).map(cue => ({
  ...cue,
  ...(cue.start !== undefined ? { start: Number(cue.start) - shift } : {}),
  ...(cue.end !== undefined ? { end: Number(cue.end) - shift } : {}),
  ...(Array.isArray(cue.words) ? { words: cue.words.map((word: any) => ({ ...word, start: Number(word?.start) - shift, end: Number(word?.end) - shift })) } : {})
}));

function importText(command: any): CaptionCue[] {
  const parsed = parseCaptions(String(command.text ?? ''), (command.format ?? 'auto') as CaptionFormat | 'auto');
  if (!parsed.cues.length) throw new Error('No captions could be read from that text. Use SRT or WebVTT.');
  return parsed.cues;
}

function compHeight(PM: any): number {
  return Number(PM.curComp?.()?.h ?? PM.proj?.h) || 1080;
}

function styleFrom(PM: any, raw: unknown, current?: CaptionStyle): CaptionStyle {
  const patch = raw && typeof raw === 'object' && !Array.isArray(raw) ? raw as Partial<CaptionStyle> : {};
  if (!current) {
    const { preset, ...rest } = patch;
    return patchStyle(presetStyle(typeof preset === 'string' ? preset : 'classic', compHeight(PM)), rest, compHeight(PM));
  }
  return patchStyle(current, patch, compHeight(PM));
}

/**
 * Keep the clip long enough to show the cues an edit placed. Only cues the
 * edit added or retimed count (every cue for a wholesale replace): cues the
 * user hid by trimming the Out point stay hidden when an earlier cue is
 * retyped or restyled.
 */
function fitDuration(PM: any, layer: any, before: readonly CaptionCue[] | null): void {
  let end = 0;
  if (!before) end = captionsEnd(layer.d.cues);
  else {
    const unchanged = new Set<CaptionCue>(before);
    let previous: Map<string, CaptionCue> | null = null;
    for (const cue of layer.d.cues as CaptionCue[]) {
      // Untouched cues are the same objects; only look up the rest by id.
      if (unchanged.has(cue) || cue.end <= end) continue;
      previous ??= new Map(before.map(item => [item.id, item]));
      const old = previous.get(cue.id);
      if (!old || old.start !== cue.start || old.end !== cue.end) end = cue.end;
    }
  }
  if (end > layer.dur) layer.dur = Math.round(end * 1e6) / 1e6;
  PM.ProjectIndex?.invalidate?.();
}

const normalized = new WeakSet<object>();

const CONTENT_KEYS = new Set(['cues', 'style', 'language']);

/**
 * set_content on a captions layer (the timeline's trim/split re-basing, source
 * transforms, scripts, the agent). The result is always normalised, so cues
 * stay sorted, disjoint and well-formed whatever the patch held; a partial
 * style merges into the current one. Other fields are refused.
 */
export function setCaptionsContent(layer: any, patch: Record<string, unknown>): void {
  for (const key of Object.keys(patch)) {
    if (!CONTENT_KEYS.has(key)) throw new Error(`Captions content field “${key}” is not editable; use edit_captions`);
  }
  const next: Record<string, unknown> = { ...layer.d, ...patch };
  if (patch.style && typeof patch.style === 'object' && !Array.isArray(patch.style)) next.style = { ...layer.d?.style, ...patch.style };
  layer.d = normalizeCaptionsContent(next);
  normalized.add(layer.d.cues);
}

export function addCaptions({ PM }: Context, command: any) {
  const from = command.from == null ? 0 : Math.max(0, Number(command.from) || 0);
  const offset = Number(command.offset) || 0;
  const source = command.text != null ? importText(command).map(cue => ({ ...cue, start: cue.start + offset, end: cue.end + offset }))
    : toLayerTime(command.cues, -offset);
  const cues = normalizeCues(toLayerTime(source as CueInput[], from));
  const layer = PM.mkLayer('captions', { name: command.name || 'Captions' });
  layer.d = normalizeCaptionsContent({ cues, style: styleFrom(PM, command.style), ...(command.language ? { language: command.language } : {}) });
  normalized.add(layer.d.cues);
  if (command.id != null) {
    if (PM.L(command.id)) throw new Error(`Layer id already exists: ${command.id}`);
    layer.id = String(command.id);
  }
  layer.from = from;
  const comp = PM.curComp?.() ?? PM.proj;
  layer.dur = command.duration != null
    ? Math.max(1 / (comp.fps || 30), Number(command.duration) || 0)
    : Math.max(0.1, (comp.dur || 0) - from, captionsEnd(cues));
  // Captions sit above the picture, like a caption track above V1.
  PM.addLayer(layer, command.index == null ? 0 : PM.clamp(Math.round(command.index), 0, PM.proj.layers.length));
  if (command.select !== false) PM.selectLayers(layer.id);
  return { id: layer.id, name: layer.name, cues: cues.length };
}

export function editCaptions({ PM, findLayer }: Context, command: any) {
  const layer = findLayer(command.target);
  if (!layer) throw new Error('Captions layer not found');
  if (layer.type !== 'captions') throw new Error(`“${layer.name}” is not a captions layer`);
  /* Content is normalised once per cue list; every edit below produces a
     normal list again, so a live drag does not re-normalise thousands of
     cues on each pointer move. */
  if (!normalized.has(layer.d?.cues)) layer.d = normalizeCaptionsContent(layer.d);
  const shift = Number(layer.from) || 0;
  const cues: CaptionCue[] = layer.d.cues;
  const op = command.op as CaptionEditOp;
  let result: Record<string, unknown> = { id: layer.id };
  switch (op) {
    case 'replace':
      layer.d.cues = normalizeCues(toLayerTime(command.cues, shift));
      break;
    case 'insert':
      layer.d.cues = insertCues(cues, toLayerTime(command.cues, shift) as any);
      break;
    case 'update': {
      const known = new Set(cues.map(cue => cue.id));
      const patches = toLayerTime(command.cues, shift) as CuePatch[];
      const missing = patches.find(patch => !known.has(String(patch.id)));
      if (missing) throw new Error(`Cue not found: ${String(missing.id)}`);
      layer.d.cues = updateCues(cues, patches.map(patch => ({ ...patch, id: String(patch.id) })));
      break;
    }
    case 'delete':
      layer.d.cues = deleteCues(cues, command.ids ?? []);
      break;
    case 'split': {
      const split = splitCue(cues, String(command.id), Number(command.at) - shift, cueId);
      if (!split.tailId) throw new Error('Split a cue at a time inside it, at least one frame from its edges');
      layer.d.cues = split.cues;
      result = { ...result, tailId: split.tailId };
      break;
    }
    case 'merge': {
      const merged = mergeCues(cues, command.ids ?? []);
      if (!merged.id) throw new Error('Merge needs at least two cues');
      layer.d.cues = merged.cues;
      result = { ...result, cue: merged.id };
      break;
    }
    case 'move':
      layer.d.cues = moveCues(cues, command.ids ?? cues.map(cue => cue.id), Number(command.by) || 0);
      break;
    case 'import': {
      const offset = Number(command.offset) || 0;
      const imported = importText(command).map(cue => ({ ...cue, start: cue.start + offset - shift, end: cue.end + offset - shift }));
      layer.d.cues = command.replace === false ? insertCues(cues, imported) : normalizeCues(imported);
      break;
    }
    case 'style':
      layer.d.style = styleFrom(PM, command.style, layer.d.style);
      break;
    default:
      throw new Error(`Unknown captions operation: ${String(op)}`);
  }
  if (command.language !== undefined) {
    if (command.language) layer.d.language = String(command.language); else delete layer.d.language;
    layer.d = normalizeCaptionsContent(layer.d);
  }
  normalized.add(layer.d.cues);
  fitDuration(PM, layer, op === 'replace' || (op === 'import' && command.replace !== false) ? null : cues);
  PM.touch?.();
  return { ...result, cues: layer.d.cues.length };
}

/** Sidecar text for a captions layer, in composition time. */
export function exportCaptionsText(layer: any, format: CaptionFormat, range?: { from?: number; to?: number; rebase?: number }): string {
  const content = normalizeCaptionsContent(layer?.d);
  const shown = { from: Math.max(range?.from ?? -Infinity, layer.from), to: Math.min(range?.to ?? Infinity, layer.from + layer.dur) };
  return formatCaptions(format, content.cues, {
    offset: Number(layer.from) || 0, ...shown, rebase: range?.rebase ?? 0,
    ...(content.language ? { language: content.language } : {})
  });
}
