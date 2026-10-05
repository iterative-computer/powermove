import type { IpcMain, IpcMainInvokeEvent } from 'electron';

import { isRecord, isString, IpcValidationError } from '../../shared/guards';
import {
  TRANSCRIPTION_IPC,
  TRANSCRIPTION_MODEL_MISSING,
  type TranscribeRequest,
  type TranscribeResult,
  type TranscriptionStatus
} from '../../shared/transcription';
import { TranscriptionAbortedError, validateRequest } from './engine';
import { LANGUAGE_TAG, type ModelStore } from './models';
import type { TranscriptionService } from './service';

/*
 * IPC for TRANSCRIPTION_IPC. Every handler checks its sender (the editor's
 * or onboarding's own document) and narrows its payload before anything
 * touches the disk. Model management answers with the fresh status; the
 * transcribe call answers with a result envelope so the error code survives.
 */

export interface TranscriptionIpcDeps {
  service: TranscriptionService;
  models: ModelStore;
  isTrustedSender(event: IpcMainInvokeEvent): boolean;
  reveal(dir: string): Promise<void> | void;
}

const MODEL_ID = /^[a-z0-9][a-z0-9._-]{0,63}$/;

function modelId(channel: string, value: unknown): string {
  if (!isString(value, 64) || !MODEL_ID.test(value)) throw new IpcValidationError(channel, 'expected a model id');
  return value;
}

export function transcribeRequest(value: unknown): TranscribeRequest {
  if (!isRecord(value)) throw new IpcValidationError(TRANSCRIPTION_IPC.transcribe, 'expected a request');
  const { path, start, end, language, requestId } = value;
  if (!isString(path, 4096)) throw new IpcValidationError(TRANSCRIPTION_IPC.transcribe, 'expected a path');
  return validateRequest({
    path,
    ...(start !== undefined ? { start: start as number } : {}),
    ...(end !== undefined ? { end: end as number } : {}),
    ...(language !== undefined ? { language: language as string } : {}),
    ...(requestId !== undefined ? { requestId: requestId as string } : {})
  });
}

export function registerTranscriptionIpc(ipc: Pick<IpcMain, 'handle'>, deps: TranscriptionIpcDeps): void {
  const guard = (event: IpcMainInvokeEvent, channel: string) => {
    if (!deps.isTrustedSender(event)) throw new Error(`${channel}: unauthorized sender`);
  };
  const status = async (): Promise<TranscriptionStatus> => {
    await deps.models.load();
    return deps.models.status();
  };
  /* Transcriptions in flight, by sender and request id, for cancel. */
  const running = new Map<string, AbortController>();

  ipc.handle(TRANSCRIPTION_IPC.status, async (event) => {
    guard(event, TRANSCRIPTION_IPC.status);
    return deps.service.status();
  });

  ipc.handle(TRANSCRIPTION_IPC.download, async (event, id: unknown) => {
    guard(event, TRANSCRIPTION_IPC.download);
    const model = modelId(TRANSCRIPTION_IPC.download, id);
    /* Downloads outlive the window that asked: progress arrives as statusChanged. */
    void deps.models.download(model).catch((error: unknown) => console.warn('[transcription] download failed', error instanceof Error ? error.message : error));
    return status();
  });

  ipc.handle(TRANSCRIPTION_IPC.cancelDownload, async (event, id: unknown) => {
    guard(event, TRANSCRIPTION_IPC.cancelDownload);
    await deps.models.cancel(modelId(TRANSCRIPTION_IPC.cancelDownload, id));
    return status();
  });

  ipc.handle(TRANSCRIPTION_IPC.remove, async (event, id: unknown) => {
    guard(event, TRANSCRIPTION_IPC.remove);
    await deps.models.remove(modelId(TRANSCRIPTION_IPC.remove, id));
    return status();
  });

  ipc.handle(TRANSCRIPTION_IPC.setActive, async (event, id: unknown) => {
    guard(event, TRANSCRIPTION_IPC.setActive);
    await deps.models.setActive(modelId(TRANSCRIPTION_IPC.setActive, id));
    return status();
  });

  ipc.handle(TRANSCRIPTION_IPC.setLanguage, async (event, language: unknown) => {
    guard(event, TRANSCRIPTION_IPC.setLanguage);
    if (!isString(language, 24) || !LANGUAGE_TAG.test(language)) throw new IpcValidationError(TRANSCRIPTION_IPC.setLanguage, 'expected a language tag or auto');
    await deps.models.setLanguage(language);
    return status();
  });

  ipc.handle(TRANSCRIPTION_IPC.reveal, async (event) => {
    guard(event, TRANSCRIPTION_IPC.reveal);
    await deps.models.load();
    await deps.reveal(deps.models.root);
  });

  ipc.handle(TRANSCRIPTION_IPC.transcribe, async (event, raw: unknown): Promise<TranscribeResult> => {
    guard(event, TRANSCRIPTION_IPC.transcribe);
    let request: TranscribeRequest;
    try {
      request = transcribeRequest(raw);
    } catch (error) {
      return { ok: false, message: error instanceof Error ? error.message : 'Invalid transcription request.' };
    }
    const controller = new AbortController();
    const slot = request.requestId ? `${event.sender.id}:${request.requestId}` : null;
    if (slot) {
      running.get(slot)?.abort();
      running.set(slot, controller);
    }
    const sender = event.sender;
    const abortOnClose = () => controller.abort();
    sender.once('destroyed', abortOnClose);
    try {
      const transcript = await deps.service.transcribe(request, (progress) => {
        if (request.requestId && !sender.isDestroyed()) sender.send(TRANSCRIPTION_IPC.progress, { requestId: request.requestId, progress });
      }, controller.signal);
      return { ok: true, transcript };
    } catch (error) {
      const code = (error as { code?: unknown })?.code;
      if (error instanceof TranscriptionAbortedError) return { ok: false, code: 'aborted', message: error.message };
      return {
        ok: false,
        ...(code === TRANSCRIPTION_MODEL_MISSING ? { code } : {}),
        message: error instanceof Error ? error.message : 'Transcription failed.'
      };
    } finally {
      sender.removeListener('destroyed', abortOnClose);
      if (slot && running.get(slot) === controller) running.delete(slot);
    }
  });

  ipc.handle(TRANSCRIPTION_IPC.cancelTranscribe, async (event, requestId: unknown) => {
    guard(event, TRANSCRIPTION_IPC.cancelTranscribe);
    if (!isString(requestId, 128)) throw new IpcValidationError(TRANSCRIPTION_IPC.cancelTranscribe, 'expected a request id');
    running.get(`${event.sender.id}:${requestId}`)?.abort();
  });
}
