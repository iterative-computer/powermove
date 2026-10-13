import type { PMRegistry } from '../registry';
import { readLocalMediaSource } from '../core/cloud-media';
import { createFolder, deleteFolder, folderList, isWithin, moveItems, renameFolder, validFolder } from '../core/asset-folders';

const MAX_FILE_BYTES = 512 * 1024 * 1024;
const revision = (PM: PMRegistry) => Math.max(0, Number(PM.proj?.revision) || 0);
const name = (value: unknown): string => {
  if (typeof value !== 'string' || !value.trim() || value.length > 160) throw new Error('Provide a name of 1–160 characters.');
  return value.trim();
};

/** Include nested compositions and structured model/texture references. */
export function mediaReferences(project: any, assetId: string): Array<{ compositionId: string; layer: any; locked: boolean }> {
  const references: Array<{ compositionId: string; layer: any; locked: boolean }> = [];
  const seen = new Set<any>();
  const walk = (comp: any) => {
    if (!comp || seen.has(comp)) return;
    seen.add(comp);
    const layers: any[] = comp.layers || [];
    const byId = new Map(layers.map(layer => [layer.id, layer]));
    const protectedLayer = (layer: any, visiting = new Set<string>()): boolean => {
      if (!layer || visiting.has(layer.id)) return false;
      visiting.add(layer.id);
      return !!layer.lock || !!layer.locked || protectedLayer(byId.get(layer.group), visiting) || protectedLayer(byId.get(layer.parent), visiting);
    };
    const uses = (value: any): boolean => {
      if (!value || typeof value !== 'object') return false;
      if (value.asset === assetId || value.assetId === assetId) return true;
      if (value.maps && Object.values(value.maps).includes(assetId)) return true;
      return Object.values(value).some(child => child && typeof child === 'object' && uses(child));
    };
    for (const layer of layers) if (uses(layer.d)) references.push({ compositionId: comp.id, layer, locked: protectedLayer(layer) });
    for (const child of Object.values(comp.comps || {})) walk(child);
  };
  walk(project);
  return references;
}

export function listMedia(PM: PMRegistry, args: Record<string, unknown>) {
  const offset = Number(args.offset ?? 0), limit = Number(args.limit ?? 100);
  if (!Number.isInteger(offset) || offset < 0 || !Number.isInteger(limit) || limit < 1 || limit > 200) throw new Error('Use offset >= 0 and limit between 1 and 200.');
  const assets = Object.values<any>(PM.proj.assets || {}).map(meta => {
    const references = mediaReferences(PM.proj, meta.id);
    const runtime = PM.assets?.get?.(meta.id);
    const error = PM.assets?.errors?.get?.(meta.id);
    const cloud = PM.assets?.cloud?.get?.(meta.id);
    const status = PM.assets?.loading?.has?.(meta.id) ? 'loading' : error ? 'failed' : runtime ? 'available' : cloud ? 'cloud' : 'missing';
    return { id: meta.id, name: meta.name, kind: meta.kind, size: meta.size, width: meta.w, height: meta.h, duration: meta.dur,
      folder: validFolder(PM.proj, meta.folder), sourcePath: meta.sourcePath || meta.path || null,
      fingerprint: meta.fingerprint || null, persisted: meta.persisted === true, status, ...(error ? { error } : {}),
      referenceCount: references.length, lockedReferences: references.filter(ref => ref.locked).length,
      references: references.slice(0, 50).map(ref => ({ compositionId: ref.compositionId, layerId: ref.layer.id, locked: ref.locked })) };
  }).filter(meta => (!args.assetId || meta.id === args.assetId) && (!args.missingOnly || ['missing', 'failed', 'cloud'].includes(meta.status)));
  return { assets: assets.slice(offset, offset + limit), total: assets.length, nextOffset: offset + limit < assets.length ? offset + limit : null, folders: folderList(PM.proj), revision: revision(PM) };
}

/** Only file bytes cross native IPC, in bounded chunks; never extension source. */
export async function importMedia(PM: PMRegistry, args: Record<string, unknown>, replace: boolean, active: () => boolean = () => true) {
  const project = PM.proj, baseRevision = revision(PM);
  const baseAnimation = PM.animVersion?.();
  const historyMark = JSON.stringify(PM.hist.mark?.());
  const deadline = Date.now() + 110_000;
  const assetId = typeof args.assetId === 'string' ? args.assetId : '';
  const meta = replace ? project.assets?.[assetId] : null;
  if (replace && !meta) throw new Error('Choose an existing assetId from list_media.');
  if (args.folderId !== undefined && args.folderId !== null && !validFolder(project, args.folderId)) throw new Error('Choose an existing media folder.');
  if (typeof args.path !== 'string' || args.path.length > 4096 || args.path.includes('\0') || !/^(\/|[A-Za-z]:[\\/])/.test(args.path)) throw new Error('Provide an absolute local media file path.');
  const path = args.path;
  const assertCurrent = () => {
    if (!active() || Date.now() > deadline || PM.animVersion?.() !== baseAnimation || JSON.stringify(PM.hist.mark?.()) !== historyMark || PM.proj !== project || revision(PM) !== baseRevision || (replace && project.assets?.[assetId] !== meta)) throw new Error('Media operation stopped because the project or agent run changed. Read get_project_state before retrying.');
    if (replace && mediaReferences(project, assetId).some(ref => ref.locked)) throw new Error('Unlock layers using this media before replacing it.');
  };
  assertCurrent();
  if (replace && PM.assets.loading?.has?.(assetId)) throw new Error('This media is still loading. Wait before replacing it.');
  if (replace && args.missingOnly === true && PM.assets.get(assetId)) return { status: 'already_available', assetId, persisted: meta.persisted === true };
  const file = await readLocalMediaSource({ id: assetId || 'agent-import', name: path.split(/[\\/]/).pop()!, sourcePath: path }, () => {
    try { assertCurrent(); return true; } catch { return false; }
  }, MAX_FILE_BYTES);
  assertCurrent();
  if (!file) throw new Error('The source is unavailable locally. Download cloud files first; no media was changed.');
  const fingerprint = await PM.MediaImport.fingerprint(file);
  assertCurrent();
  if (args.expectedFingerprint !== undefined && args.expectedFingerprint !== fingerprint) throw new Error('The chosen file does not match expectedFingerprint; no media was changed.');
  if (!replace) {
    const existing = Object.values<any>(project.assets || {}).find(item => item.fingerprint === fingerprint && PM.assets.get(item.id) && item.persisted);
    if (existing) return { status: 'already_imported', assetId: existing.id, persisted: true };
  }
  const result = replace
    ? await PM.assets.replace(assetId, file, { assertCurrent })
    : (await PM.assets.add(file, { assertCurrent, requirePersisted: true, newAsset: true })).importResult;
  const id = result.asset.id;
  if (!result.persisted || !PM.assets.get(id) || !project.assets?.[id]) throw new Error('Media import was not verified in durable storage and the live registry.');
  if (!replace && args.folderId) project.assets[id].folder = args.folderId;
  PM.bus.emit('assets');
  PM.invalidate?.('all');
  PM.autosave?.();
  return { status: replace ? 'replaced' : 'imported', assetId: id, name: project.assets[id].name, persisted: true, fingerprint, revision: revision(PM) };
}

export function manageMedia(PM: PMRegistry, args: Record<string, unknown>) {
  const project = PM.proj;
  const operation = args.operation;
  const ids = args.assetIds === undefined ? [] : args.assetIds;
  if (!Array.isArray(ids) || ids.length > 100 || ids.some(id => typeof id !== 'string' || !project.assets?.[id])) throw new Error('Choose at most 100 existing assetIds from list_media.');
  const unique = [...new Set<string>(ids)];
  const folderId = args.folderId;
  const targetFolder = folderId === null || folderId === undefined ? null : validFolder(project, folderId);
  if (folderId != null && !targetFolder) throw new Error('Choose an existing media folder.');
  if (['rename', 'move', 'delete'].includes(String(operation)) && !unique.length) throw new Error('Supply assetIds.');
  if (operation === 'rename' && unique.length !== 1) throw new Error('Rename one asset at a time.');
  if (['rename', 'create_folder', 'rename_folder'].includes(String(operation))) name(args.name);
  if (['rename_folder', 'delete_folder'].includes(String(operation)) && !targetFolder) throw new Error('Supply an existing folderId.');
  if (operation === 'move_folder') {
    if (!targetFolder) throw new Error('Supply the folderId to move.');
    if (args.parentId != null && !validFolder(project, args.parentId)) throw new Error('Choose an existing parentId.');
    if (args.parentId && isWithin(project, String(args.parentId), targetFolder)) throw new Error('A folder cannot move inside itself or a descendant.');
  }
  if (operation === 'delete') for (const id of unique) {
    const refs = mediaReferences(project, id);
    if (refs.some(ref => ref.locked)) throw new Error('Unlock layers using this media before deleting it.');
    const preview = JSON.parse(JSON.stringify(project));
    PM.MediaImport.removeAsset(preview, id, { preserveRuntime: true });
    if (mediaReferences(preview, id).length) throw new Error('This media has structured references that cannot be safely removed. Remove those references before deleting it.');
    if (refs.length && args.removeReferenced !== true) throw new Error('This media is used by layers. Set removeReferenced: true only when the user asked to remove those layers too.');
  }
  if (!['rename', 'move', 'delete', 'create_folder', 'rename_folder', 'move_folder', 'delete_folder'].includes(String(operation))) throw new Error('Unknown media management operation.');
  let createdFolder: string | undefined;
  let removedLayerIds: string[] = [];
  const beforeHistory = JSON.stringify(PM.hist.mark?.());
  PM.hist.do('Manage media', () => {
    if (operation === 'rename') { project.assets[unique[0]!].name = name(args.name); }
    if (operation === 'move') moveItems(project, { assets: unique }, targetFolder);
    if (operation === 'delete') for (const id of unique) removedLayerIds.push(...PM.MediaImport.removeAsset(project, id).removedLayerIds);
    if (operation === 'create_folder') createdFolder = createFolder(project, name(args.name), targetFolder);
    if (operation === 'rename_folder') renameFolder(project, targetFolder!, name(args.name));
    if (operation === 'move_folder') moveItems(project, { folders: [targetFolder!] }, (args.parentId as string | null) ?? null);
    if (operation === 'delete_folder') deleteFolder(project, targetFolder!);
  });
  PM.sel.layers = PM.sel.layers.filter((id: string) => !removedLayerIds.includes(id));
  for (const event of ['assets', 'layers', 'sel', 'project']) PM.bus.emit(event);
  PM.invalidate?.('all');
  PM.autosave?.();
  return { operation, changed: JSON.stringify(PM.hist.mark?.()) !== beforeHistory, assetIds: unique, ...(createdFolder ? { folderId: createdFolder } : {}), removedLayerIds, revision: revision(PM) };
}
