import {
  TRANSCRIPTION_MODEL_MISSING,
  type TranscribeRequest,
  type Transcript,
  type TranscriptionStatus
} from '../../../shared/transcription';
import { modelFor, type ModelNeeds } from './format';
import { transcriptionBridge } from './host';

export { transcriptionBridge };

/*
 * Renderer seam for transcription. Captions and any panel call these; the
 * transcription engine lane owns the implementation (bridge wiring, the
 * download sheet). Contract:
 *
 * - `ensureTranscriptionModel(reason, needs?)` resolves true once a model is
 *   ready (with `{ wordTimestamps: true }`, one that times each word, as
 *   captions need). With none it opens the download sheet (a Settings-style
 *   sheet with the model picker) and resolves when the user finishes the
 *   download, or false if they dismiss it.
 * - `transcribe` rejects with an error whose `code` is
 *   TRANSCRIPTION_MODEL_MISSING when no model is ready. Prefer calling
 *   `ensureTranscriptionModel` first.
 */

const UNAVAILABLE: TranscriptionStatus = { models: [], activeModelId: null, available: false };

export async function transcriptionStatus(): Promise<TranscriptionStatus> {
  const host = transcriptionBridge();
  if (!host) return { ...UNAVAILABLE, models: [] };
  try {
    return await host.status();
  } catch {
    return { ...UNAVAILABLE, models: [] };
  }
}

export async function ensureTranscriptionModel(reason: string, needs: ModelNeeds = {}): Promise<boolean> {
  const status = await transcriptionStatus();
  if (status.available === false) return false;
  if (modelFor(status, needs)) return true;
  // The sheet is Svelte UI; loading it on demand keeps it out of headless
  // importers of this seam (the agent harness in the document engine).
  const { openModelSheet } = await import('./sheet');
  return openModelSheet(reason, status, needs);
}

function failure(message: string, code?: string): Error {
  const error = new Error(message) as Error & { code?: string };
  if (code) error.code = code;
  if (code === 'aborted') error.name = 'AbortError';
  return error;
}

let sequence = 0;

export async function transcribe(
  request: TranscribeRequest,
  onProgress?: (progress: number) => void,
  signal?: AbortSignal
): Promise<Transcript> {
  const host = transcriptionBridge();
  if (!host) throw failure('Transcription is not available here.', TRANSCRIPTION_MODEL_MISSING);
  if (signal?.aborted) throw failure('Transcription was cancelled.', 'aborted');
  const requestId = request.requestId ?? `tr-${Date.now().toString(36)}-${++sequence}`;
  const offProgress = onProgress
    ? host.onProgress((event) => { if (event.requestId === requestId) onProgress(event.progress); })
    : null;
  const abort = () => { void host.cancelTranscribe(requestId).catch(() => undefined); };
  signal?.addEventListener('abort', abort, { once: true });
  try {
    const result = await host.transcribe({ ...request, requestId });
    if (result.ok) return result.transcript;
    throw failure(result.message, result.code);
  } finally {
    offProgress?.();
    signal?.removeEventListener('abort', abort);
  }
}
