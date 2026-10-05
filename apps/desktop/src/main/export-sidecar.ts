/*
 * Caption sidecars written next to an exported video ("Film.mp4" →
 * "Film.srt" or "Film.en.vtt"). The renderer can only name a video this
 * window itself just exported through a save dialog; the sidecar shares its
 * folder and base name, and only .srt / .vtt text is ever written.
 */
import path from 'node:path';
import type { IpcMain } from 'electron';
import { IPC, SIDECAR_SUFFIX_PATTERN, type ExportSidecarRequest, type FileSaveResult } from '../shared/ipc';
import { atomicWrite } from './project-files';

const MAX_SIDECAR_BYTES = 32 * 1024 * 1024;
const RECENT = 16;
const exported = new Map<number, string[]>();

/** Called once a save dialog's export has been written for `owner`. */
export function rememberExport(owner: number, filePath: string): void {
  const list = (exported.get(owner) ?? []).filter(item => item !== filePath);
  list.push(filePath);
  exported.set(owner, list.slice(-RECENT));
}

export function forgetExports(owner: number): void {
  exported.delete(owner);
}

export function sidecarPath(videoPath: string, extension: 'srt' | 'vtt', suffix?: string): string {
  const base = path.basename(videoPath, path.extname(videoPath));
  const tag = suffix && SIDECAR_SUFFIX_PATTERN.test(suffix) ? `.${suffix}` : '';
  return path.join(path.dirname(videoPath), `${base}${tag}.${extension}`);
}

export async function writeSidecar(owner: number, request: unknown): Promise<FileSaveResult> {
  const value = (request && typeof request === 'object' ? request : {}) as Partial<ExportSidecarRequest>;
  if (typeof value.path !== 'string' || !(exported.get(owner) ?? []).includes(value.path)) {
    return { ok: false, cancelled: false, error: 'Captions can only be written next to a video exported from this window.' };
  }
  if (value.extension !== 'srt' && value.extension !== 'vtt') return { ok: false, cancelled: false, error: 'Unsupported caption format.' };
  if (typeof value.text !== 'string') return { ok: false, cancelled: false, error: 'Caption text is missing.' };
  // A bad suffix is refused rather than dropped, so two layers' files can
  // never silently land on the same unsuffixed name.
  if (value.suffix !== undefined && (typeof value.suffix !== 'string' || !SIDECAR_SUFFIX_PATTERN.test(value.suffix))) {
    return { ok: false, cancelled: false, error: 'Invalid caption file name.' };
  }
  const bytes = new TextEncoder().encode(value.text);
  if (bytes.byteLength > MAX_SIDECAR_BYTES) return { ok: false, cancelled: false, error: 'The caption file is too large.' };
  const target = sidecarPath(value.path, value.extension, value.suffix);
  try {
    await atomicWrite(target, bytes);
    return { ok: true, path: target };
  } catch (error: any) {
    return { ok: false, cancelled: false, error: error?.message || 'Could not write the caption file.' };
  }
}

export function registerExportSidecar(ipcMain: Pick<IpcMain, 'handle'>, ctx: { isTrustedSender(event: any): boolean }): void {
  ipcMain.handle(IPC.exportSidecar, async (event, request: unknown) => {
    if (!ctx.isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    return writeSidecar(event.sender.id, request);
  });
}
