import {
  TRANSCRIPTION_MODEL_MISSING,
  type TranscribeRequest,
  type Transcript,
  type TranscriptionStatus
} from '../../shared/transcription';

/*
 * Main-process seam for transcription. The engine installs the real service
 * at startup; agent media tools and IPC handlers only ever call
 * `transcriptionService()`, so they never depend on the engine's internals.
 */

export class TranscriptionModelMissingError extends Error {
  readonly code = TRANSCRIPTION_MODEL_MISSING;
  constructor(message = 'No transcription model is downloaded yet.') { super(message); }
}

export interface TranscriptionService {
  status(): Promise<TranscriptionStatus>;
  /** Rejects with TranscriptionModelMissingError when no model is ready. */
  transcribe(request: TranscribeRequest, onProgress?: (progress: number) => void, signal?: AbortSignal): Promise<Transcript>;
  /** Ask the focused editor window to show the download sheet. */
  requestModel(reason: string): void;
}

const unavailable: TranscriptionService = {
  status: async () => ({ models: [], activeModelId: null }),
  transcribe: async () => { throw new TranscriptionModelMissingError(); },
  requestModel: () => {}
};

let current: TranscriptionService = unavailable;

export function transcriptionService(): TranscriptionService { return current; }
export function setTranscriptionService(service: TranscriptionService): void { current = service; }
