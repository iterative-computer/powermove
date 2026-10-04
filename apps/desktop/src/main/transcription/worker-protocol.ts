import type { TranscriptSegment } from '../../shared/transcription';

/* Messages between main and the transcription process (worker.ts). */

export type WorkerRequest =
  | {
    type: 'transcribe';
    id: string;
    file: string;
    start?: number;
    end?: number;
    modelDir: string;
    threads: number;
    ffmpeg: string;
  }
  | { type: 'cancel'; id: string }
  /** Load the model ahead of the first request. */
  | { type: 'warm'; modelDir: string; threads: number };

export type WorkerResponse =
  | { type: 'ready' }
  | { type: 'progress'; id: string; progress: number }
  | { type: 'done'; id: string; duration: number; segments: TranscriptSegment[] }
  | { type: 'error'; id: string; message: string }
  | { type: 'cancelled'; id: string };
