/*
 * `media.assets.source` and `media.assets.audio`: the replacements for the
 * deprecated live fields `url`, `el` and `audioBlob` (plan section 26,
 * amendment 23). They read the same live asset the compatibility fields are
 * on, so both forms agree, and wait for it to load instead of returning a
 * field that is not set yet.
 */
import type { AssetSource } from './api';

type LegacyPM = Record<string, any>;

export const DEFAULT_WAIT_MS = 30_000;
const POLL_MS = 100;
/** Object URLs made for audio assets, kept while their live asset is. */
const audioUrls = new WeakMap<object, string>();

/** Resolves with `pick(live)` once it is not undefined, null when the asset is
 *  gone or `pick` says so, or null after `timeoutMs`. */
function waitForLive<T>(PM: LegacyPM, id: string, timeoutMs: number, pick: (live: any) => T | null | undefined): Promise<T | null> {
  return new Promise((resolve) => {
    let done = false;
    let poll: ReturnType<typeof setInterval> | null = null;
    let timer: ReturnType<typeof setTimeout> | null = null;
    let off: (() => void) | null = null;
    const finish = (value: T | null) => {
      if (done) return;
      done = true;
      if (poll) clearInterval(poll);
      if (timer) clearTimeout(timer);
      off?.();
      resolve(value);
    };
    const check = () => {
      if (!PM?.proj?.assets?.[id]) return finish(null);
      const live = PM?.assets?.get?.(id);
      if (!live) return;
      if (live.error || live.failed) return finish(null);
      const value = pick(live);
      if (value !== undefined) finish(value);
    };
    check();
    if (done) return;
    const unsubscribe = PM?.bus?.on?.('assets', check);
    off = typeof unsubscribe === 'function' ? unsubscribe : null;
    // Renditions and decoded audio land without an `assets` event: poll too.
    poll = setInterval(check, POLL_MS);
    timer = setTimeout(() => finish(null), Math.max(0, timeoutMs));
  });
}

function timeout(options?: { timeoutMs?: number }): number {
  const ms = Number(options?.timeoutMs);
  return Number.isFinite(ms) && ms >= 0 ? Math.min(ms, 10 * 60_000) : DEFAULT_WAIT_MS;
}

export function assetSource(PM: LegacyPM, id: string, options?: { timeoutMs?: number }): Promise<AssetSource | null> {
  if (typeof id !== 'string' || !id) return Promise.resolve(null);
  return waitForLive<AssetSource>(PM, id, timeout(options), (live) => {
    const kind = live.kind;
    if (kind === 'model') return null;
    if (kind !== 'image' && kind !== 'video' && kind !== 'audio') return null;
    let url = String(live.url || live.el?.currentSrc || live.el?.src || '');
    let blob: Blob | null = live.blob instanceof Blob ? live.blob : null;
    if (!url && kind === 'audio') {
      // An audio asset has no element: it plays from its bytes, one URL per loaded asset.
      blob = live.audioBlob instanceof Blob ? live.audioBlob : blob;
      if (!blob) return undefined;
      url = audioUrls.get(live) ?? URL.createObjectURL(blob);
      audioUrls.set(live, url);
    }
    if (!url) return undefined;
    return { url, kind, mime: blob?.type || null, rendition: !!live.playbackProxy };
  });
}

export function assetAudio(PM: LegacyPM, id: string, options?: { timeoutMs?: number }): Promise<Blob | null> {
  if (typeof id !== 'string' || !id) return Promise.resolve(null);
  return waitForLive<Blob>(PM, id, timeout(options), (live) => {
    if (live.audioBlob instanceof Blob) return live.audioBlob;
    if (live.kind === 'audio' && live.blob instanceof Blob) return live.blob;
    if (live.kind === 'video' && live.hasAudio === false) return null;
    if (live.kind !== 'audio' && live.kind !== 'video') return null;
    return undefined;
  });
}
