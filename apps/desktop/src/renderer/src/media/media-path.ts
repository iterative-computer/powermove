import { bridge as hostBridge } from '../kernel/bridge';
import {
  MEDIA_STAGE_CHUNK_BYTES,
  type MediaPathBridge,
  type MediaPathOrigin,
  type MediaPathStageRequest
} from '../../../shared/media-tools';

/*
 * Renderer seam: turn a project media asset into an absolute file path the
 * main process (or the remote host) can read with ffmpeg or the
 * transcription engine. The media-tools lane owns the implementation:
 * the original source when it still exists, otherwise the asset's bytes
 * staged once to a cache file (desktop) or uploaded to the host (web bridge).
 *
 * Rejects when the asset does not exist or has no audio/video bytes.
 */
export async function resolveMediaPath(assetId: string): Promise<string> {
  return (await resolveMediaSource(assetId)).path;
}

export interface ResolvedMediaSource {
  path: string;
  origin: MediaPathOrigin;
  assetId: string;
  name: string;
  kind: 'video' | 'audio';
  /** The bytes behind `path` are Powermove's playback proxy, not the original file. */
  proxy: boolean;
}

type Registry = Record<string, any>;
const registry = (): Registry | undefined => (globalThis as { window?: { PM?: Registry } }).window?.PM;

/* One staging per asset at a time: captions and the agent may ask together. */
const inflight = new Map<string, Promise<ResolvedMediaSource>>();
/* Projects this window resolved media for, so their staged files can be let go. */
const usedProjects = new Set<string>();
let watching = false;

/** Same as resolveMediaPath, plus where the path came from. */
export function resolveMediaSource(assetId: string): Promise<ResolvedMediaSource> {
  const PM = registry();
  const projectId = typeof PM?.proj?.id === 'string' ? PM.proj.id : '';
  const key = `${projectId}\n${assetId}`;
  const pending = inflight.get(key);
  if (pending) return pending;
  const job = resolve(PM, assetId).finally(() => inflight.delete(key));
  inflight.set(key, job);
  return job;
}

async function resolve(PM: Registry | undefined, assetId: string): Promise<ResolvedMediaSource> {
  const meta = PM?.proj?.assets?.[assetId];
  if (!PM || !meta) throw new Error(`Media asset ${assetId} does not exist in this project.`);
  if (meta.kind !== 'video' && meta.kind !== 'audio') throw new Error(`“${meta.name || assetId}” is ${meta.kind === 'image' ? 'an image' : `a ${meta.kind}`} asset, not audio or video.`);
  const media = hostBridge()?.mediaPath as MediaPathBridge | undefined;
  if (!media) throw new Error('This Powermove host cannot hand media files to tools.');
  const projectId = typeof PM.proj.id === 'string' && PM.proj.id ? PM.proj.id : undefined;
  watchProjectChanges(PM, media, projectId);
  const runtime = PM.assets?.get?.(assetId);
  const name = String(meta.name || assetId);
  const describe = (path: string, origin: MediaPathOrigin, proxy: boolean): ResolvedMediaSource =>
    ({ path, origin, assetId, name, kind: meta.kind, proxy });

  /* An image sequence's "source" is one frame of many; only its stored movie stands for it. */
  const sourcePath = meta.imageSequence ? undefined : (meta.sourcePath || meta.path || undefined);
  const storedProxy = meta.playbackProxy === true || runtime?.playbackProxy === true;
  const found = await media.lookup({
    assetId,
    ...(projectId ? { projectId } : {}),
    ...(typeof sourcePath === 'string' ? { sourcePath } : {}),
    ...(typeof meta.fingerprint === 'string' ? { fingerprint: meta.fingerprint } : {}),
    ...(typeof meta.storageKey === 'string' ? { storageKey: meta.storageKey } : {}),
    ...(Number(meta.size) > 0 ? { size: Number(meta.size) } : {})
  });
  if (found) return describe(found.path, found.origin, found.origin !== 'source' && storedProxy);

  let blob: Blob | null = null;
  try { blob = (await PM.MediaStore?.get?.(meta)) ?? null; } catch { blob = null; }
  if (!blob || !blob.size) {
    const error = PM.assets?.errors?.get?.(assetId);
    throw new Error(`“${name}” has no media bytes in this project${error ? ` (${error})` : ''}. Relink it with Locate File, then try again.`);
  }
  const request: MediaPathStageRequest = {
    assetId, name, size: blob.size,
    ...(projectId ? { projectId } : {}),
    ...(typeof meta.storageKey === 'string' ? { storageKey: meta.storageKey } : {}),
    ...(blob.type ? { type: blob.type } : {})
  };
  const staged = await stage(media, blob, request);
  return describe(staged.path, staged.origin, storedProxy);
}

async function stage(media: MediaPathBridge, blob: Blob, request: MediaPathStageRequest) {
  if (media.stageFile) {
    const file = blob instanceof File ? blob : new File([blob], request.name, { type: blob.type });
    return media.stageFile(file, request);
  }
  if (!media.stageBegin || !media.stageChunk || !media.stageFinish) throw new Error('This Powermove host cannot stage media for tools.');
  const begun = await media.stageBegin(request);
  if ('path' in begun) return begun;
  try {
    for (let offset = 0; offset < blob.size; offset += MEDIA_STAGE_CHUNK_BYTES) {
      const data = new Uint8Array(await blob.slice(offset, offset + MEDIA_STAGE_CHUNK_BYTES).arrayBuffer());
      await media.stageChunk(begun.token, offset, data);
    }
    return await media.stageFinish(begun.token);
  } catch (error) {
    await media.stageAbort?.(begun.token).catch(() => undefined);
    throw error;
  }
}

/* Staged files belong to the project that asked for them, and live as long as
   this window holds that project: on screen or parked as a tab. Switching tabs
   keeps them; closing the tab, trashing the project or moving it to another
   window lets main drop them (the window closing or the app quitting drops
   everything in main). */
function watchProjectChanges(PM: Registry, media: MediaPathBridge, projectId: string | undefined): void {
  if (projectId) usedProjects.add(projectId);
  if (watching || typeof PM.bus?.on !== 'function') return;
  watching = true;
  PM.bus.on('projects:open', () => {
    const held = heldProjects(PM);
    for (const id of [...usedProjects]) {
      if (held.has(id)) continue;
      usedProjects.delete(id);
      void media.release?.(id).catch(() => undefined);
    }
  });
}

/** The projects this window still has open: the one on screen plus its tabs. */
function heldProjects(PM: Registry): Set<string> {
  const held = new Set<string>();
  if (typeof PM.proj?.id === 'string') held.add(PM.proj.id);
  try {
    const tabs: unknown = PM.Tabs?.list?.();
    if (Array.isArray(tabs)) for (const id of tabs) if (typeof id === 'string') held.add(id);
  } catch { /* no tab strip (document engine): the project on screen is all */ }
  return held;
}

/** Test seam. */
export function resetMediaPathForTests(): void {
  inflight.clear();
  usedProjects.clear();
  watching = false;
}
