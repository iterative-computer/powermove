/** Agent `import_media` and `replace_media` in a document host worker (the
 * macOS hidden host). The worker cannot decode media, and an asset's record is
 * what the editor's decoder writes: durations, sizes, rates, poster, SVG paths,
 * converted formats and image sequences (legacy/gl/raster.ts). So the host runs
 * today's import in its project-pinned editor tab (server/native/browser.ts),
 * which stores the bytes in the host media store and commits the record to the
 * shared session; the worker's harness keeps the agent run's transaction, as it
 * does for any media tool. Nothing here builds an asset record by hand. */
import type { DocumentHostServices } from './protocol';

export function installWorkerImports(PM: any, run: NonNullable<DocumentHostServices['mediaImport']>['run'], projectId: () => string | null): void {
  PM.HostImportMedia = async (args: Record<string, unknown>, replace: boolean, assertCurrent: () => void) => {
    const project = projectId();
    if (!project) throw new Error('Open a project before importing media.');
    assertCurrent();
    const result = await run(project, args, replace) as Record<string, any>;
    // The tab's commit reaches this worker (as a session patch) before the
    // host's answer does; a missing record would be a sync failure, never a
    // reason to answer as if the import succeeded.
    const assetId = typeof result?.assetId === 'string' ? result.assetId : '';
    const meta = assetId ? PM.proj?.assets?.[assetId] : null;
    if (!meta) throw new Error('media-import-unsynced: The import finished in the editor tab, but its media did not reach this project. Read list_media before retrying.');
    if ((result.status === 'imported' || result.status === 'replaced') && meta.fingerprint !== result.fingerprint) {
      throw new Error('media-import-unsynced: The imported media record did not reach this project. Read list_media before retrying.');
    }
    return { ...result, ...(typeof result.revision === 'number' ? { revision: Math.max(0, Number(PM.proj?.revision) || 0) } : {}) };
  };
}
