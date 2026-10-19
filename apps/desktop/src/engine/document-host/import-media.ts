/** The production harness owns cancellation, locked references and the agent
 * transaction. This adapter only prepares through the host and commits the
 * same persisted asset identity; immutable content hashes preserve Undo bytes. */
import { assetIdentity } from '../../shared/media-identity';
import { validFolder } from '../../renderer/src/legacy/core/asset-folders';
import { writeDocument } from '../../renderer/src/engine-doc/write';
import type { DocumentHostServices } from './protocol';
import type { WorkerMedia } from './media';

export function installWorkerImports(PM: any, media: WorkerMedia, prepare: NonNullable<DocumentHostServices['mediaImport']>['prepare']): void {
  PM.HostImportMedia = async (args: Record<string, unknown>, replace: boolean, assertCurrent: () => void) => {
    const project = PM.proj, assetId = replace ? String(args.assetId) : null, previous = assetId ? project.assets?.[assetId] : null;
    if (replace && args.missingOnly === true && media.get(assetId!)) return { status: 'already_available', assetId, persisted: previous.persisted === true };
    const prepared = await prepare({ path: String(args.path), ...(args.storage === undefined ? {} : { storage: String(args.storage) }) });
    assertCurrent();
    if (args.expectedFingerprint !== undefined && args.expectedFingerprint !== prepared.fingerprint) throw new Error('The chosen file does not match expectedFingerprint; no media was changed.');
    if (replace && prepared.kind !== previous.kind) throw new Error(`Choose a ${previous.kind} file to replace this media.`);
    if (!replace) {
      const existing = Object.values<any>(project.assets ?? {}).find(meta => meta.fingerprint === prepared.fingerprint && meta.contentHash === prepared.prepared.contentHash && meta.persisted && media.get(meta.id));
      if (existing) return { status: 'already_imported', assetId: existing.id, persisted: true };
    }
    const id = assetId ?? PM.uid('a');
    const next = assetIdentity(id, { name: prepared.sourcePath.split(/[\\/]/).pop() }, prepared.kind, prepared.prepared,
      prepared.fingerprint, prepared.storageKey, prepared.sourcePath, true, previous?.layerDefinition);
    const folder = replace ? previous.folder : validFolder(project, args.folderId);
    if (folder) (next as any).folder = folder;
    const apply = (value: unknown) => {
      writeDocument(PM, [['assets', id]], () => { project.assets[id] = JSON.parse(JSON.stringify(value)); });
      PM.touch(); PM.Projects?.put?.(project);
      for (const event of ['assets', 'layers', 'project']) PM.bus.emit(event);
      PM.invalidate?.('all'); PM.autosave?.();
    };
    if (replace) {
      const before = JSON.parse(JSON.stringify(previous));
      apply(next);
      if (!PM.hist.external(`Replace ${previous.name || 'media'}`, () => apply(before), () => apply(next), { bytes: Number(before.size || 0) + Number(next.size || 0) })) {
        apply(before); throw new Error('Could not add the replacement to Undo history; the original file is unchanged.');
      }
    } else apply(next);
    // The just-committed identity is available immediately; the session patch
    // follows before the answer, after which host lookups can also see it.
    media.map.set(id, { id, dur: next.dur, w: next.w, h: next.h, hasAudio: prepared.kind === 'audio' || next.hasAudio === true });
    media.loading.delete(id);
    return { status: replace ? 'replaced' : 'imported', assetId: id, name: next.name, persisted: true,
      fingerprint: prepared.fingerprint, storage: (next as any).storage === 'linked' ? 'linked' : 'copied', revision: Number(project.revision) || 0 };
  };
}
