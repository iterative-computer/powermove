import type { PMRegistry } from '../registry';
import { isProperty } from '../core/content-properties';
import { layerTiming } from './media-source';

/*
 * `check_project`: structural lint for the agent, in the spirit of dapi's
 * `check`. Only what the editor itself can prove: frames where no layer
 * draws, layers that never show, missing or failed media, clips that play
 * past their media, and audio pushed over full scale by its own gain. It
 * evaluates the composition with the same active/opacity rules the
 * compositor uses, frame by frame, so keyframed opacity and group visibility
 * count exactly. Nothing is guessed from geometry.
 */

export interface CheckIssue {
  code: string;
  severity: 'error' | 'warning';
  message: string;
  layerId?: string;
  /** Composition seconds, [start, end). */
  ranges?: Array<[number, number]>;
}

export interface CheckReport {
  ok: boolean;
  span: [number, number];
  fps: number;
  /** Frames are evaluated every N frames on very long, very layered comps. */
  sampledEveryFrames: number;
  layers: number;
  mediaChecked: boolean;
  issues: CheckIssue[];
  truncated?: number;
}

const NON_DRAWING = new Set(['audio', 'null', 'group', 'adjustment']);
const MAX_EVALUATIONS = 400_000;
const MAX_ISSUES = 60;

const r3 = (value: number): number => Math.round(value * 1000) / 1000;

function staticOn(layer: any): boolean | null {
  if (isProperty(layer.on)) return layer.on.kf.length || layer.on.expr ? null : !!layer.on.v;
  return layer.on !== false;
}

export function checkProject(PM: PMRegistry): CheckReport {
  const project = PM.proj;
  const fps = Math.max(1, Number(project.fps) || 30);
  const frame = 1 / fps;
  const duration = Math.max(0, Number(project.dur) || 0);
  const work = Array.isArray(project.work) ? project.work.map(Number) : [0, duration];
  const span: [number, number] = [Math.max(0, Math.min(duration, work[0] ?? 0)), Math.max(0, Math.min(duration, work[1] ?? duration))];
  if (span[1] <= span[0]) { span[0] = 0; span[1] = duration; }
  const layers: any[] = project.layers || [];
  const issues: CheckIssue[] = [];
  const runtime = PM.assets && typeof PM.assets.get === 'function' && PM.assets.map instanceof Map ? PM.assets : null;
  const assetName = (id: string) => project.assets?.[id]?.name || id;

  /* ── media ───────────────────────────────────────────── */
  const unusable = new Set<string>();
  for (const layer of layers) {
    if (!['video', 'audio', 'image'].includes(layer.type)) continue;
    const id = layer.d?.asset;
    if (!id) {
      unusable.add(layer.id);
      issues.push({ code: 'no-media', severity: 'warning', layerId: layer.id, message: `“${layer.name}” has no media attached, so it draws nothing.` });
      continue;
    }
    if (!project.assets?.[id]) {
      unusable.add(layer.id);
      issues.push({ code: 'missing-media', severity: 'error', layerId: layer.id, message: `“${layer.name}” points at media ${id}, which is not in the project.` });
      continue;
    }
    if (!runtime || runtime.get(id) || runtime.loading?.has?.(id)) continue;
    unusable.add(layer.id);
    const failure = runtime.errors?.get?.(id);
    const cloud = runtime.cloud?.get?.(id);
    issues.push(failure
      ? { code: 'media-failed', severity: 'error', layerId: layer.id, message: `“${assetName(id)}” failed to load: ${failure}` }
      : cloud
        ? { code: 'media-in-cloud', severity: 'warning', layerId: layer.id, message: `“${assetName(id)}” is in ${cloud.provider} and not downloaded, so “${layer.name}” is blank until it is.` }
        : { code: 'media-offline', severity: 'error', layerId: layer.id, message: `“${assetName(id)}” is offline; “${layer.name}” is blank until the file is relinked.` });
  }

  /* ── timing and media length ─────────────────────────── */
  const inComposition: any[] = [];
  for (const layer of layers) {
    const from = Number(layer.from), dur = Number(layer.dur);
    if (!(dur > 0)) {
      issues.push({ code: 'zero-duration', severity: 'warning', layerId: layer.id, message: `“${layer.name}” has no duration, so it never plays.` });
      continue;
    }
    if (from >= duration - 1e-6 || from + dur <= 1e-6) {
      issues.push({ code: 'outside-composition', severity: 'warning', layerId: layer.id, message: `“${layer.name}” plays ${r3(from)}s–${r3(from + dur)}s, entirely outside the ${r3(duration)}s composition.` });
      continue;
    }
    inComposition.push(layer);
    if ((layer.type === 'video' || layer.type === 'audio') && !unusable.has(layer.id) && !project.assets?.[layer.d.asset]?.imageSequence) {
      const media = runtime?.get?.(layer.d.asset);
      const length = Number(media?.dur ?? project.assets?.[layer.d.asset]?.dur);
      if (!(length > 0)) continue;
      const timing = layerTiming(PM, layer);
      if (!timing.constant) continue;
      const [, lastSource] = timing.samples[1]!;
      const over = lastSource - length;
      if (over > frame * 1.5) {
        const startsOver = layer.from + Math.max(0, (length - timing.samples[0]![1]) / Math.max(1e-9, (lastSource - timing.samples[0]![1]) / layer.dur));
        issues.push({ code: 'past-media-end', severity: 'warning', layerId: layer.id, ranges: [[r3(startsOver), r3(layer.from + layer.dur)]],
          message: `“${layer.name}” runs ${r3(over)}s past the end of “${assetName(layer.d.asset)}”; nothing new plays after ${r3(startsOver)}s.` });
      }
    }
  }

  /* ── coverage: which frames show a layer at all ──────── */
  const matteSources = new Set(layers.map((layer) => layer.matteSource).filter(Boolean));
  const visual = inComposition.filter((layer) => !NON_DRAWING.has(layer.type)
    && PM.TYPE_META?.[layer.type]?.visual !== false && !matteSources.has(layer.id));
  const drawsNothing = (layer: any): boolean => unusable.has(layer.id)
    || (layer.type === 'text' && typeof layer.d?.text === 'string' && !layer.d.text.trim() && !isProperty(layer.d.text))
    || (layer.type === 'precomp' && (!layer.d?.comp || !project.comps?.[layer.d.comp]));
  for (const layer of visual) if (layer.type === 'precomp' && drawsNothing(layer)) {
    issues.push({ code: 'missing-composition', severity: 'error', layerId: layer.id, message: `“${layer.name}” nests a composition that does not exist.` });
  }
  const frames = Math.max(1, Math.ceil((span[1] - span[0]) * fps - 1e-6));
  const stride = Math.max(1, Math.ceil(frames * Math.max(1, visual.length) / MAX_EVALUATIONS));
  const seenActive = new Set<string>(), seenVisible = new Set<string>();
  const gaps: Array<[number, number]> = [];
  let gapStart: number | null = null;
  for (let index = 0; index < frames; index += stride) {
    const time = span[0] + index * frame;
    let covered = false;
    for (const layer of visual) {
      if (!PM.active(layer, time)) continue;
      seenActive.add(layer.id);
      if ((PM.worldOpacity?.(layer, time) ?? PM.ev(layer, 'opacity', time) / 100) <= 0.001) continue;
      seenVisible.add(layer.id);
      if (!drawsNothing(layer)) covered = true;
    }
    if (!covered && gapStart === null) gapStart = time;
    if (covered && gapStart !== null) { gaps.push([r3(gapStart), r3(time)]); gapStart = null; }
  }
  if (gapStart !== null) gaps.push([r3(gapStart), r3(span[1])]);

  if (gaps.length) {
    const total = gaps.reduce((sum, [start, end]) => sum + (end - start), 0);
    const fill = PM.normalizeFill?.(project.backgroundFill, project.bg);
    const background = fill?.type === 'none' ? 'transparent frames' : `only the background${fill?.type === 'solid' ? ` (${fill.stops?.[0]?.color ?? project.bg})` : ''}`;
    const all = gaps.length === 1 && gaps[0]![0] <= span[0] + 1e-6 && gaps[0]![1] >= span[1] - 1e-6;
    issues.unshift({
      code: all ? 'no-visuals' : 'empty-frames', severity: 'error', ranges: gaps.slice(0, 40),
      message: all
        ? `No layer is visible anywhere in ${r3(span[0])}s–${r3(span[1])}s; every frame shows ${background}.`
        : `No layer is visible in ${gaps.length} ${gaps.length === 1 ? 'span' : 'spans'} totaling ${r3(total)}s; those frames show ${background}.`
    });
  }

  for (const layer of visual) {
    if (seenVisible.has(layer.id) || drawsNothing(layer)) continue;
    if (!seenActive.has(layer.id)) {
      // Hidden on purpose (eye off, hidden group) is the user's call; only a span outside the checked range is news.
      if (staticOn(layer) !== false && !(PM.groupAncestors?.(layer) || []).some((group: any) => staticOn(group) === false)
        && (layer.from >= span[1] || layer.from + layer.dur <= span[0])) {
        issues.push({ code: 'outside-work-area', severity: 'warning', layerId: layer.id, message: `“${layer.name}” plays ${r3(layer.from)}s–${r3(layer.from + layer.dur)}s, outside the checked work area ${r3(span[0])}s–${r3(span[1])}s.` });
      }
      continue;
    }
    issues.push({ code: 'never-visible', severity: 'warning', layerId: layer.id, message: `“${layer.name}” is fully transparent wherever it plays in ${r3(span[0])}s–${r3(span[1])}s (its opacity or its group's opacity is 0), so it never shows.` });
  }

  /* ── audio over full scale from its own gain ─────────── */
  if (runtime) for (const layer of inComposition) {
    const audible = layer.type === 'audio' || (layer.type === 'video' && layer.d?.embeddedAudio === true);
    if (!audible || unusable.has(layer.id)) continue;
    const media = runtime.get(layer.d.asset);
    const peaks: ArrayLike<number> | null = media?.peaks ?? null;
    const length = Number(media?.audioBuffer?.duration ?? media?.audioDur ?? media?.dur);
    if (!peaks || !peaks.length || !(length > 0)) continue;
    const gain = layer.type === 'audio' ? maxValue(layer.d?.gain, 1) : 1;
    const timing = layerTiming(PM, layer);
    const sources = timing.samples.map(([, time]) => time);
    const from = Math.max(0, Math.floor(Math.min(...sources) / length * peaks.length));
    const to = Math.min(peaks.length, Math.ceil(Math.max(...sources) / length * peaks.length));
    let peak = 0;
    for (let index = from; index < to; index++) peak = Math.max(peak, Number(peaks[index]) || 0);
    const level = peak * gain;
    if (level > 1.001) {
      issues.push({ code: 'audio-clipping', severity: 'warning', layerId: layer.id,
        message: `“${layer.name}” peaks at +${(20 * Math.log10(level)).toFixed(1)} dBFS with gain ${r3(gain)}; it will clip. Lower its gain to ${r3(Math.floor(gain / level * 1000) / 1000)} or less.` });
    }
  }

  const truncated = Math.max(0, issues.length - MAX_ISSUES);
  return {
    ok: !issues.length, span: [r3(span[0]), r3(span[1])], fps, sampledEveryFrames: stride,
    layers: layers.length, mediaChecked: !!runtime, issues: issues.slice(0, MAX_ISSUES), ...(truncated ? { truncated } : {})
  };
}

function maxValue(value: any, fallback: number): number {
  if (!isProperty(value)) return Number.isFinite(Number(value)) ? Number(value) : fallback;
  const values = [Number(value.v), ...value.kf.map((key: any) => Number(key.v))].filter(Number.isFinite);
  return values.length ? Math.max(...values) : fallback;
}
