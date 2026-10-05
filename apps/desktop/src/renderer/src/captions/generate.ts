/*
 * Captions from speech: transcribe the source media of audio/video layers,
 * map transcript time onto the composition through each clip's timing, and
 * segment the words into readable cues. The transcription engine and media
 * paths are injected seams (renderer/src/transcription/client.ts and
 * renderer/src/media/media-path.ts) so this module is testable on its own.
 */
import { TRANSCRIPTION_MODEL_MISSING, type TranscribeRequest, type Transcript } from '../../../shared/transcription';
import type { CaptionCue, CaptionWord } from './model';
import { segmentWords, type SegmentOptions } from './segment';
import { mapWordsToComposition, visibleSourceRange, type ClipTiming } from './time-map';

export interface GenerateDeps {
  ensureModel(reason: string): Promise<boolean>;
  resolvePath(assetId: string): Promise<string>;
  transcribe(request: TranscribeRequest, onProgress?: (progress: number) => void, signal?: AbortSignal): Promise<Transcript>;
  /** Composition timing of a clip (layer start, trim, speed, remap). */
  clipTiming(layer: any): ClipTiming;
  progress?(fraction: number, layer: any): void;
  signal?: AbortSignal;
  language?: string;
  segment?: Partial<SegmentOptions>;
}

export type GenerateResult =
  | { status: 'ok'; cues: CaptionCue[]; language?: string }
  | { status: 'model-missing' }
  | { status: 'empty' };

export const isModelMissing = (error: unknown) =>
  !!error && typeof error === 'object' && (error as { code?: unknown }).code === TRANSCRIPTION_MODEL_MISSING;

const cancelled = () => new DOMException('Caption generation was cancelled', 'AbortError');

export function captionableLayer(layer: any): boolean {
  return !!layer && (layer.type === 'audio' || layer.type === 'video') && typeof layer.d?.asset === 'string' && !!layer.d.asset;
}

/** Words of a transcript; a segment without word timings counts as one. */
export function transcriptWords(transcript: Transcript): CaptionWord[] {
  return transcript.segments.flatMap(segment => segment.words?.length
    ? segment.words.map(word => ({ text: word.text.trim(), start: word.start, end: word.end }))
    : [{ text: segment.text.trim(), start: segment.start, end: segment.end }])
    .filter(word => word.text);
}

export async function captionsFromSpeech(layers: any[], deps: GenerateDeps): Promise<GenerateResult> {
  const targets = layers.filter(captionableLayer);
  if (!targets.length) throw new Error('Select an audio or video clip to caption.');
  const reason = targets.length === 1 ? `Captions for “${targets[0].name}”` : `Captions for ${targets.length} clips`;
  if (!await deps.ensureModel(reason)) return { status: 'model-missing' };
  const words: CaptionWord[] = [];
  let language: string | undefined;
  for (const [index, layer] of targets.entries()) {
    if (deps.signal?.aborted) throw cancelled();
    const clip = deps.clipTiming(layer);
    const [start, end] = visibleSourceRange(clip);
    const path = await deps.resolvePath(layer.d.asset);
    let transcript: Transcript;
    try {
      transcript = await deps.transcribe(
        { path, start, end, requestId: `captions-${layer.id}`, ...(deps.language ? { language: deps.language } : {}) },
        progress => deps.progress?.((index + Math.max(0, Math.min(1, progress))) / targets.length, layer),
        deps.signal
      );
    } catch (error) {
      if (isModelMissing(error)) return { status: 'model-missing' };
      throw error;
    }
    // An engine may resolve (with a partial or empty transcript) after Cancel.
    if (deps.signal?.aborted) throw cancelled();
    language ??= transcript.language;
    words.push(...mapWordsToComposition(transcriptWords(transcript), clip));
  }
  if (deps.signal?.aborted) throw cancelled();
  if (!words.length) return { status: 'empty' };
  const cues = segmentWords(words.sort((a, b) => a.start - b.start), deps.segment);
  return { status: 'ok', cues, ...(language ? { language } : {}) };
}
