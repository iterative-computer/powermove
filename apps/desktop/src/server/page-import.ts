/** `import_media_to_timeline` (external MCP): today's hidden-host import, in
 * an editor page. The legacy hidden host runs it in its one page; the native
 * hidden host in the project-pinned editor tab (native/mcp.ts), so the asset
 * record, poster, SVG paths, converted formats, import defaults and placement
 * are exactly what the editor's import writes (legacy/app.ts PM.importFiles). */
import { stat } from 'node:fs/promises';
import path from 'node:path';
import type { Page } from 'playwright';

export async function importFileInPage(current: Page, args: Record<string, unknown>): Promise<unknown> {
  const file = path.resolve(String(args.path));
  const info = await stat(file);
  if (info.size > 512 * 1024 * 1024) throw new Error('Local media imports are limited to 512 MiB.');
  if (!info.isFile()) throw new Error('Import a regular media file.');
  if (!/\.(?:mp4|mov|m4v|webm|mkv|avi|mp3|wav|m4a|aac|aif|aiff|flac|ogg|png|jpg|jpeg|webp|gif|svg|avif|bmp|tif|tiff|heic|heif|glb|gltf|obj|fbx|blend)$/i.test(file)) throw new Error('Unsupported media file type.');
  await current.evaluate(() => { const input = document.createElement('input'); input.type = 'file'; input.id = 'powermove-mcp-import'; input.hidden = true; document.body.appendChild(input); });
  try {
    await current.locator('#powermove-mcp-import').setInputFiles(file);
    return await current.evaluate(async ({ at, assetOnly, folderId, expectedFingerprint, sourcePath, projectId }) => {
      const PM = (window as any).PM;
      if (PM.proj.id !== projectId) throw new Error('The session project changed before import.');
      const beforeLayers = new Set(PM.proj.layers.map((layer: any) => layer.id));
      const beforeAssets = new Set(Object.keys(PM.proj.assets || {}));
      const input = document.getElementById('powermove-mcp-import') as HTMLInputElement;
      if (assetOnly) {
        const project = PM.proj, source = input.files![0]!;
        const revision = project.revision, animation = PM.animVersion?.(), history = JSON.stringify(PM.hist.mark?.());
        const assertCurrent = () => { if (PM.proj !== project || project.revision !== revision || PM.animVersion?.() !== animation || JSON.stringify(PM.hist.mark?.()) !== history) throw new Error('The project changed during import.'); };
        if (folderId !== undefined && folderId !== null && !Object.hasOwn(project.assetFolders || {}, folderId)) throw new Error('Choose an existing media folder.');
        const fingerprint = await PM.MediaImport.fingerprint(source);
        assertCurrent();
        if (expectedFingerprint !== undefined && fingerprint !== expectedFingerprint) throw new Error('The chosen file does not match expectedFingerprint.');
        const existing = Object.values<any>(project.assets || {}).find(asset => asset.fingerprint === fingerprint && asset.persisted && PM.assets.get(asset.id));
        if (existing) return { status: 'already_imported', assetId: existing.id, persisted: true };
        const result = (await PM.assets.add(source, { requirePersisted: true, newAsset: true, assertCurrent })).importResult;
        if (PM.proj !== project) throw new Error('The project changed during import.');
        const id = result.asset.id, asset = project.assets?.[id];
        if (!result.persisted || !asset || !PM.assets.get(id)) throw new Error('Media import was not verified in durable storage.');
        asset.sourcePath = sourcePath;
        if (folderId) asset.folder = folderId;
        PM.bus.emit('assets'); PM.invalidate?.('all'); PM.autosave?.();
        return { status: 'imported', assetId: id, name: asset.name, persisted: true, fingerprint };
      }
      await PM.importFiles(Array.from(input.files!), { placement: at === undefined ? null : { at } });
      const assets = Object.values<any>(PM.proj.assets || {}).filter(asset => !beforeAssets.has(asset.id)).map(asset => ({ id: asset.id, name: asset.name, kind: asset.kind }));
      const layers = PM.proj.layers.filter((layer: any) => !beforeLayers.has(layer.id)).map((layer: any) => ({ id: layer.id, name: layer.name, type: layer.type }));
      if (!assets.length && !layers.length) throw new Error('Powermove did not import the file. Inspect get_workspace_state for errors.');
      return { assets, layers };
    }, { at: args.at as number | undefined, assetOnly: args.assetOnly === true, folderId: args.folderId as string | null | undefined, expectedFingerprint: args.expectedFingerprint as string | undefined, sourcePath: file, projectId: String(args.projectId) });
  } finally { await current.locator('#powermove-mcp-import').evaluate(element => element.remove()).catch(() => {}); }
}
