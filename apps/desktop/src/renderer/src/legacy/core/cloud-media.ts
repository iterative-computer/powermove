import type { PowermoveBridge } from '../../../../shared/ipc';
import { bridge as hostBridge } from '../../kernel/bridge';

/** Recovered Files are synthesized from IPC chunks, so webUtils has no path. */
export const cloudSourcePaths = new WeakMap<File, string>();
type MediaBridge = PowermoveBridge['media'];
export interface CloudAsset { id: string; name: string; sourcePath?: string; path?: string }
export interface CloudStatus {
  provider: 'iCloud' | 'Cloud';
  state: 'offloaded' | 'downloading' | 'error';
  error?: string;
}

async function readSourceFile(media: MediaBridge, result: { token: string; size: number }, meta: CloudAsset, current: () => boolean) {
  try {
    const parts: ArrayBuffer[] = [];
    for (let offset = 0; offset < result.size;) {
      if (!current()) return null;
      const chunk = await media.readCloudSource(result.token, offset, Math.min(4 * 1024 * 1024, result.size - offset));
      if (!chunk.byteLength) throw new Error('The media file ended unexpectedly');
      const owned = new Uint8Array(chunk.byteLength);
      owned.set(chunk); parts.push(owned.buffer); offset += chunk.byteLength;
    }
    if (!current()) return null;
    const file = new window.File(parts, meta.name);
    cloudSourcePaths.set(file, meta.sourcePath || meta.path || '');
    return file;
  } finally { await media.releaseCloudSource(result.token).catch(() => undefined); }
}

/** A missing browser cache does not mean the original file is missing. */
export async function readLocalMediaSource(meta: CloudAsset, current: () => boolean): Promise<File | null> {
  const media = (hostBridge() as any)?.media as MediaBridge | undefined;
  const source = meta.sourcePath || meta.path;
  if (!source || !media?.openLocalSource || !current()) return null;
  const result = await media.openLocalSource(source);
  return result ? readSourceFile(media, result, meta, current) : null;
}

export function createCloudMedia(PM: Record<string, any>, recover: (meta: any, file: File, current: () => boolean) => Promise<void>) {
  const statuses = new Map<string, CloudStatus>();
  const pending = new Map<string, Promise<void>>();
  let generation = 0;
  const bridge = () => (hostBridge() as any)?.media as MediaBridge | undefined;
  const emit = () => PM.bus?.emit?.('assets');
  const source = (meta: CloudAsset) => meta.sourcePath || meta.path || '';
  const currentAsset = (project: any, meta: CloudAsset, epoch: number) =>
    generation === epoch && PM.proj === project && project.assets?.[meta.id] === meta;
  const controller = {
    get: (id: string) => statuses.get(id),
    clear() { generation++; statuses.clear(); pending.clear(); },
    async scan(project: any, missing: CloudAsset[]) {
      const media = bridge();
      if (!media?.cloudStatus || !media.cloudPrompt) return;
      const epoch = generation;
      const candidates = missing.filter(meta => source(meta));
      const paths = [...new Set(candidates.map(source))];
      const found: CloudAsset[] = [];
      try {
        for (let offset = 0; offset < paths.length; offset += 256) {
          const states = await media.cloudStatus(paths.slice(offset, offset + 256));
          if (PM.proj !== project || epoch !== generation) return;
          for (const meta of candidates) {
            const state = states[source(meta)];
            if (!currentAsset(project, meta, epoch) || PM.assets.get(meta.id) || statuses.has(meta.id)) continue;
            if (state !== 'icloud' && state !== 'cloud') continue;
            statuses.set(meta.id, { provider: state === 'icloud' ? 'iCloud' : 'Cloud', state: 'offloaded' });
            found.push(meta);
          }
        }
        if (!found.length) return;
        emit();
        void (async () => {
          let download = PM.store?.get?.('autoDownloadCloudMedia', false) === true;
          if (!download) {
            const decision = await media.cloudPrompt(found.map(meta => meta.name));
            if (PM.proj !== project || epoch !== generation) return;
            // Checking the box opts into future openings even if this batch is deferred.
            PM.store?.set?.('autoDownloadCloudMedia', decision.automatic === true);
            download = decision.download;
          }
          if (download) await PM.MediaImport.mapBounded(found, 3, (meta: CloudAsset) =>
            currentAsset(project, meta, epoch) ? controller.download(meta.id) : undefined);
        })().catch(() => {
          if (PM.proj === project && epoch === generation) PM.toast?.('Could not open the cloud download prompt. Use the download button on the media card.');
        });
      } catch (error) {
        if (PM.proj === project && epoch === generation) PM.toast?.('Could not check cloud media. Try downloading the file in Finder.');
      }
    },
    download(id: string): Promise<void> {
      if (pending.has(id)) return pending.get(id)!;
      const project = PM.proj;
      const meta = project?.assets?.[id] as CloudAsset | undefined;
      const status = statuses.get(id);
      const media = bridge();
      if (!meta || !status || !media?.downloadCloudSource) return Promise.resolve();
      const epoch = generation;
      const current = () => currentAsset(project, meta, epoch);
      statuses.set(id, { ...status, state: 'downloading', error: undefined });
      emit();
      const operation = (async () => {
        try {
          const result = await media.downloadCloudSource(source(meta));
          const file = await readSourceFile(media, result, meta, current);
          if (!file || !current()) return;
          await recover(meta, file, current);
          if (current()) statuses.delete(id);
        } catch (error) {
          if (!current()) return;
          const message = error instanceof Error ? error.message : 'Download failed. Try again.';
          statuses.set(id, { ...status, state: 'error', error: message });
          PM.toast?.(`Could not restore “${meta.name}” · ${message}`);
        } finally {
          if (current()) { pending.delete(id); emit(); PM.invalidate?.('render'); }
        }
      })();
      pending.set(id, operation);
      return operation;
    },
  };
  return controller;
}
