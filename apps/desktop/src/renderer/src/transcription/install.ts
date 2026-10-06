/* Editor-side transcription: answers main's modelRequested with the download
   sheet, and reports background downloads (started in onboarding, or a sheet
   that was hidden) as a progress notice like a project save. */
import type { TranscriptionModelInfo, TranscriptionStatus } from '../../../shared/transcription';
import { ensureTranscriptionModel, transcribe, transcriptionStatus } from './client';
import type { ModelNeeds } from './format';
import { transcriptionBridge } from './host';
import { isModelSheetOpen, openModelSheet, setSheetHost } from './sheet';

type PMLike = Record<string, any>;

const NOTICE = 'transcription-download';

/** What the download notice should say for a status change, or null to leave it. */
export function downloadNotice(previous: TranscriptionStatus | null, next: TranscriptionStatus):
  | { message: string; progress: number; done?: boolean; error?: boolean }
  | null {
  const before = new Map((previous?.models ?? []).map((model) => [model.id, model.state]));
  const active = next.models.find((model) => model.state === 'downloading');
  if (active) return { message: `Downloading ${active.name}`, progress: active.progress ?? 0 };
  const finished = next.models.find((model: TranscriptionModelInfo) => before.get(model.id) === 'downloading' && model.state === 'ready');
  if (finished) return { message: `${finished.name} is ready for transcription`, progress: 1, done: true };
  const failed = next.models.find((model) => before.get(model.id) === 'downloading' && model.state === 'error');
  if (failed) return { message: `${failed.name} could not be downloaded. ${failed.error ?? ''}`.trim(), progress: 0, error: true };
  return null;
}

export function installTranscription(PM: PMLike): void {
  setSheetHost(PM);
  PM.Transcription = {
    status: transcriptionStatus,
    ensureModel: ensureTranscriptionModel,
    transcribe,
    openSheet: (reason?: string, needs?: ModelNeeds) => openModelSheet(reason ?? '', null, needs ?? {})
  };
  const host = transcriptionBridge();
  if (!host) return;
  host.onModelRequested(({ reason }) => { void ensureTranscriptionModel(String(reason ?? '')); });

  let previous: TranscriptionStatus | null = null;
  let showing = false;
  host.onStatus((status) => {
    const notice = downloadNotice(previous, status);
    previous = status;
    if (!notice) return;
    /* The sheet and Settings › Transcription show progress themselves. */
    const visible = isModelSheetOpen() || PM.SettingsUI?.isOpen;
    if (notice.done || notice.error) {
      if (!showing && visible) return;
      showing = false;
      if (notice.error) PM.dismissToast?.(NOTICE);
      PM.toast?.(notice.message, notice.error ? 6000 : 2600, notice.error
        ? { error: true, corner: 'top-right' }
        : { key: NOTICE, progress: 1, completed: true, corner: 'top-right' });
      return;
    }
    if (visible || !document.hasFocus()) {
      if (showing) PM.dismissToast?.(NOTICE);
      showing = false;
      return;
    }
    showing = true;
    PM.toast?.(notice.message, 2200, { key: NOTICE, sticky: true, progress: notice.progress, corner: 'top-right' });
  });
  void host.status().then((status) => { previous = status; }).catch(() => undefined);
}
