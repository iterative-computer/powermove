import type { TranscriptSegment } from '../../shared/transcription';
import type { CatalogTiming, WordLag } from './catalog';

/* Messages between main and the transcription process (worker.ts). */

export interface WorkerModel {
  /** Absolute path of the model's GGUF file. */
  file: string;
  /** How finely the model times its output; picks the windowing and how words get their times. */
  timing: CatalogTiming;
  /** Seconds the model reports words late (CatalogModel.lag). */
  lag?: WordLag;
}

export type WorkerRequest =
  | {
    type: 'transcribe';
    id: string;
    file: string;
    start?: number;
    end?: number;
    model: WorkerModel;
    /** Language tag to hand the model; omitted lets it detect. */
    language?: string;
    threads: number;
    ffmpeg: string;
    /** Packaged app: libtranscribe.dylib outside the asar, where dlopen can map it. */
    runtime?: string;
  }
  | { type: 'cancel'; id: string }
  /** Load the model ahead of the first request, so the one-time Metal
   *  shader compile happens off the critical path. Answered with 'loaded'. */
  | { type: 'warm'; model: WorkerModel; threads: number; runtime?: string };

export type WorkerResponse =
  | { type: 'ready' }
  /** A model finished loading (the first load in a fresh app compiles the Metal shaders, ~15 s). */
  | { type: 'loaded'; file: string }
  | { type: 'progress'; id: string; progress: number }
  | { type: 'done'; id: string; duration: number; segments: TranscriptSegment[]; /** Detected by the model, when it says. */ language?: string }
  | { type: 'error'; id: string; message: string }
  | { type: 'cancelled'; id: string };
