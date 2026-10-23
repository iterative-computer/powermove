/*
 * The media status a document host worker gives the editor's document code
 * (`PM.assets`), in place of the editor's decoded media registry
 * (legacy/gl/raster.ts): an asset is available when the host's media store
 * (or its linked source) has its bytes, found through the embedder's
 * `mediaPath.lookup`, and offline when it does not. The fields document code
 * reads from a decoded asset (`dur`, `w`, `h`, `hasAudio`) come from the
 * asset record, which the editor's decoder wrote at import.
 *
 * Consumers: check_project (offline media, media length), list_media
 * (status), edit_video and agent media sources (source duration, availability).
 * The worker waits for pending lookups before a call that reads them, so an
 * answer never depends on timing.
 */
import type { MediaPathLookupRequest, MediaPathResult } from '../../shared/media-tools';

interface AssetRecord { [key: string]: unknown; id: string; kind?: string; dur?: number; w?: number; h?: number; hasAudio?: boolean; storageKey?: string; fingerprint?: string; sourcePath?: string; path?: string; size?: number }

/** What document code reads from a decoded asset. */
export interface MediaView extends AssetRecord { hasAudio: boolean }

const KINDS = new Set(['video', 'audio', 'image', 'model']);

export class WorkerMedia {
  readonly map = new Map<string, MediaView>();
  readonly errors = new Map<string, string>();
  readonly loading = new Set<string>();
  readonly cloud = new Map<string, { provider: string }>();
  /** asset id → the record fields the lookup was made for. */
  private readonly checked = new Map<string, string>();
  private pending: Promise<unknown> = Promise.resolve();

  constructor(
    private readonly lookup: (request: MediaPathLookupRequest) => Promise<MediaPathResult | null>,
    private readonly projectId: () => string | null,
    private readonly log: (line: string) => void,
  ) {}

  get(id: string): MediaView | undefined { return this.map.get(id); }

  /** Importing and replacing media decodes bytes, which a document host does not do itself (survey C24). */
  add(): Promise<never> { return Promise.reject(new Error('Importing media is not available in the document host yet; import it in the editor.')); }
  replace(): Promise<never> { return Promise.reject(new Error('Replacing media is not available in the document host yet; replace it in the editor.')); }

  /** Looks up every media asset whose record changed since it was checked; drops removed ones. */
  refresh(assets: Record<string, AssetRecord> | undefined): void {
    const live = new Set<string>();
    const jobs: Array<Promise<void>> = [];
    for (const meta of Object.values(assets ?? {})) {
      if (!meta || typeof meta.id !== 'string' || !KINDS.has(String(meta.kind))) continue;
      live.add(meta.id);
      // Format, SVG/editable-path, poster, animation and model metadata are
      // document facts too. A replacement can change them without changing size.
      const key = JSON.stringify(meta);
      if (this.checked.get(meta.id) === key) continue;
      this.checked.set(meta.id, key);
      this.loading.add(meta.id);
      const request: MediaPathLookupRequest = {
        assetId: meta.id,
        ...(this.projectId() ? { projectId: this.projectId()! } : {}),
        ...(typeof (meta.sourcePath ?? meta.path) === 'string' ? { sourcePath: meta.sourcePath ?? meta.path } : {}),
        ...(typeof meta.fingerprint === 'string' ? { fingerprint: meta.fingerprint } : {}),
        ...(typeof meta.storageKey === 'string' ? { storageKey: meta.storageKey } : {}),
        ...(typeof meta.size === 'number' ? { size: meta.size } : {}),
      };
      jobs.push(this.lookup(request).then((found) => {
        if (this.checked.get(meta.id) !== key) return;
        if (found?.path) this.map.set(meta.id, { ...meta, hasAudio: meta.kind === 'audio' || meta.hasAudio === true });
        else this.map.delete(meta.id);
      }, (error) => {
        if (this.checked.get(meta.id) !== key) return;
        // No lookup service, or it failed: the media cannot be confirmed, so it is offline here.
        this.map.delete(meta.id);
        this.log(`[engine] media ${meta.id}: ${(error as Error)?.message ?? error}`);
      }).finally(() => { if (this.checked.get(meta.id) === key) this.loading.delete(meta.id); }));
    }
    for (const id of [...this.checked.keys()]) if (!live.has(id)) { this.checked.delete(id); this.map.delete(id); this.loading.delete(id); }
    if (jobs.length) this.pending = Promise.all([this.pending, ...jobs]);
  }

  /** Resolves once every lookup started so far has answered. */
  async settled(): Promise<void> {
    for (let last: Promise<unknown> | null = null; last !== this.pending;) { last = this.pending; await last; }
  }
}
