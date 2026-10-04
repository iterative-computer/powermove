import { TRANSCRIPTION_MODEL_MISSING, type TranscribeRequest, type Transcript, type TranscriptionStatus } from '../../../shared/transcription';

/*
 * Renderer seam for transcription. Captions and any panel call these; the
 * transcription engine lane owns the implementation (bridge wiring, the
 * download sheet). Contract:
 *
 * - `ensureTranscriptionModel(reason)` resolves true once a model is ready.
 *   With none ready it opens the download sheet (a Settings-style sheet with
 *   the model picker) and resolves when the user finishes the download, or
 *   false if they dismiss it.
 * - `transcribe` rejects with an error whose `code` is
 *   TRANSCRIPTION_MODEL_MISSING when no model is ready. Prefer calling
 *   `ensureTranscriptionModel` first.
 */

export async function transcriptionStatus(): Promise<TranscriptionStatus> {
  return { models: [], activeModelId: null };
}

export async function ensureTranscriptionModel(_reason: string): Promise<boolean> {
  return false;
}

export async function transcribe(
  _request: TranscribeRequest,
  _onProgress?: (progress: number) => void,
  _signal?: AbortSignal
): Promise<Transcript> {
  throw Object.assign(new Error('Transcription is not available yet.'), { code: TRANSCRIPTION_MODEL_MISSING });
}
