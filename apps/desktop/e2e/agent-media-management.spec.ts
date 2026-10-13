import { writeFile } from 'node:fs/promises';
import path from 'node:path';
import { expect, test } from './helpers/app';
import { importFixture, fixturePath } from './helpers/media';

// Hidden Electron, isolated profile, real native file reading and media storage.
test('agent replaces missing media directly, preserves animation, verifies storage, and supports Undo/retry', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  const original = path.join(session.userData, 'original.png');
  const replacement = path.join(session.userData, 'replacement.png');
  for (const [file, color] of [[original, '#ff0000'], [replacement, '#0000ff']]) {
    const bytes = await page.evaluate(color => {
      const canvas = document.createElement('canvas'); canvas.width = canvas.height = 64;
      const context = canvas.getContext('2d')!; context.fillStyle = color; context.fillRect(0, 0, 64, 64);
      return canvas.toDataURL('image/png').split(',')[1];
    }, color!);
    await writeFile(file!, Buffer.from(bytes!, 'base64'));
  }
  const chooser = page.waitForEvent('filechooser');
  await page.evaluate(() => (window as any).PM.pickFiles());
  await (await chooser).setFiles(original);
  await page.waitForFunction(() => (window as any).PM.proj.layers.some((layer: any) => layer.name === 'original.png'));
  const result = await page.evaluate(async replacement => {
    const PM = (window as any).PM;
    const tool = async (name: string, args: any = {}) => {
      const reply = await PM.AgentHarness.test.handleLiveAgentTool({ runId: 'media-recovery', callId: 'media-test', tool: name, arguments: args, baseRevision: PM.proj.revision });
      return JSON.parse(reply.content[0].text);
    };
    const layer = PM.proj.layers.find((item: any) => item.name === 'original.png');
    PM.proj.layers = [layer];
    PM.Edit.apply({ type: 'replace_keyframes', target: layer.id, path: 'rotation.z', keyframes: [{ time: 0, value: 0 }, { time: 2, value: 45 }], preserveHandEdits: false });
    const id = layer.d.asset, layers = JSON.stringify(PM.proj.layers);
    const oldKey = PM.proj.assets[id].storageKey;
    // Reproduce the offline asset with surviving project references.
    PM.assets.clear(); PM.assets.errors.set(id, 'Old source unavailable');
    const before = await tool('list_media', { missingOnly: true });
    const replaced = await tool('replace_media', { assetId: id, path: replacement, missingOnly: true });
    const pixel = () => { const frame = PM.renderFrameTo(0, PM.proj.w, PM.proj.h); return [...frame.getContext('2d').getImageData(frame.width / 2, frame.height / 2, 1, 1).data]; };
    const blue = pixel();
    const cached = !!await PM.MediaStore.get(PM.proj.assets[id]);
    const after = await tool('list_media', { assetId: id });
    const history = JSON.stringify(PM.hist.mark());
    const retry = await tool('replace_media', { assetId: id, path: replacement, missingOnly: true });
    const retryDidNotAddUndo = history === JSON.stringify(PM.hist.mark());
    await tool('__finish_run', { commit: true });
    PM.hist.undo(); const undoKey = PM.proj.assets[id].storageKey;
    PM.hist.redo(); const redoBlue = pixel();
    return { id, before: before.assets[0].status, replaced, after: after.assets[0].status, layersUnchanged: layers === JSON.stringify(PM.proj.layers), blue, cached, retry, retryDidNotAddUndo, undoRestoredMetadata: undoKey === oldKey, redoBlue, errorsCleared: !PM.assets.errors.has(id), sourcePath: PM.proj.assets[id].sourcePath };
  }, replacement);
  expect(result).toMatchObject({ before: 'failed', after: 'available', layersUnchanged: true, cached: true, blue: [0, 0, 255, 255], redoBlue: [0, 0, 255, 255], undoRestoredMetadata: true, retryDidNotAddUndo: true, errorsCleared: true, sourcePath: replacement });
  expect(result.replaced).toMatchObject({ assetId: result.id, persisted: true, status: 'replaced' });
  expect(result.retry.status).toBe('already_available');
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('agent imports without timeline layers and manages folders/deletion with native Undo and lock guards', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  await importFixture(page, 'tone.wav');
  await page.waitForFunction(() => (window as any).PM.proj.layers.some((layer: any) => layer.name === 'tone.wav'));
  const result = await page.evaluate(async source => {
    const PM = (window as any).PM;
    const tool = async (name: string, args: any = {}) => {
      const reply = await PM.AgentHarness.test.handleLiveAgentTool({ runId: 'media-manage', callId: 'media-test', tool: name, arguments: args, baseRevision: PM.proj.revision });
      return JSON.parse(reply.content[0].text);
    };
    const failure = async (name: string, args: any) => { try { await tool(name, args); return ''; } catch (error) { return String(error); } };
    const count = PM.proj.layers.length;
    const imported = await tool('import_media', { path: source });
    const noNewLayers = count === PM.proj.layers.length;
    const retry = await tool('import_media', { path: source });
    const folder = await tool('manage_media', { operation: 'create_folder', name: 'Recovered' });
    await tool('manage_media', { operation: 'move', assetIds: [imported.assetId], folderId: folder.folderId });
    await tool('manage_media', { operation: 'rename', assetIds: [imported.assetId], name: 'Music' });
    const moved = PM.proj.assets[imported.assetId].folder === folder.folderId;
    const renamed = PM.proj.assets[imported.assetId].name === 'Music';
    PM.hist.undo(); const undoName = PM.proj.assets[imported.assetId].name;
    await tool('list_media');
    const used = PM.proj.layers.find((layer: any) => layer.name === 'tone.wav');
    used.lock = true;
    const lockError = await failure('replace_media', { assetId: used.d.asset, path: source });
    const lockedDelete = await failure('manage_media', { operation: 'delete', assetIds: [used.d.asset], removeReferenced: true });
    used.lock = false;
    const referencedDelete = await failure('manage_media', { operation: 'delete', assetIds: [used.d.asset] });
    const wrongKind = await failure('replace_media', { assetId: used.d.asset, path: source.replace('tone.mp3', 'animated.gif') });
    const deleteFolder = await tool('manage_media', { operation: 'delete_folder', folderId: folder.folderId });
    const contentsKept = !!PM.proj.assets[imported.assetId] && !PM.proj.assets[imported.assetId].folder;
    await tool('manage_media', { operation: 'delete', assetIds: [imported.assetId] });
    const deleted = !PM.proj.assets[imported.assetId];
    PM.hist.undo(); const undoDelete = !!PM.proj.assets[imported.assetId];
    return { noNewLayers, imported, retry, moved, renamed, undoName, lockError, lockedDelete, referencedDelete, wrongKind, deleteFolder, contentsKept, deleted, undoDelete };
  }, fixturePath('tone.mp3'));
  expect(result).toMatchObject({ noNewLayers: true, moved: true, renamed: true, undoName: 'tone.mp3', contentsKept: true, deleted: true, undoDelete: true });
  expect(result.imported.persisted).toBe(true);
  expect(result.retry).toMatchObject({ status: 'already_imported', assetId: result.imported.assetId });
  expect(result.lockError).toContain('Unlock');
  expect(result.lockedDelete).toContain('Unlock');
  expect(result.referencedDelete).toContain('removeReferenced');
  expect(result.wrongKind).toContain('audio');
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('agent replacement failures and interrupted work leave the original intact and can be retried', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  await importFixture(page, 'tone.wav');
  await page.waitForFunction(() => (window as any).PM.proj.layers.some((layer: any) => layer.name === 'tone.wav'));
  const result = await page.evaluate(async source => {
    const PM = (window as any).PM;
    const layer = PM.proj.layers.find((item: any) => item.name === 'tone.wav');
    const id = layer.d.asset, original = JSON.stringify(PM.proj.assets[id]), runtime = PM.assets.get(id);
    const request = (tool: string, args: any, runId = 'media-failure') => PM.AgentHarness.test.handleLiveAgentTool({ runId, callId: 'test', tool, arguments: args, baseRevision: PM.proj.revision });
    const fail = async (action: Promise<any>) => { try { await action; return ''; } catch (error) { return String(error); } };
    const unchanged = () => JSON.stringify(PM.proj.assets[id]) === original && PM.assets.get(id) === runtime;
    const fingerprintError = await fail(request('replace_media', { assetId: id, path: source, expectedFingerprint: 'v2:wrong' }));
    const realPut = PM.MediaStore.put;
    PM.MediaStore.put = async () => false;
    const storageError = await fail(request('replace_media', { assetId: id, path: source }));
    PM.MediaStore.put = realPut;
    const afterStorage = unchanged();
    // Hold the real durable write while another edit advances the revision.
    let release!: () => void, began!: () => void;
    const gate = new Promise<void>(resolve => { release = resolve; });
    const started = new Promise<void>(resolve => { began = resolve; });
    PM.MediaStore.put = async (...args: any[]) => { began(); await gate; return realPut(...args); };
    const replacing = request('replace_media', { assetId: id, path: source });
    await started; PM.touch(); release();
    const conflictError = await fail(replacing);
    PM.MediaStore.put = realPut;
    const afterConflict = unchanged();
    await request('list_media', {});
    // Stopping the run while bytes are being stored must prevent publication.
    const gate2 = new Promise<void>(resolve => { release = resolve; });
    const started2 = new Promise<void>(resolve => { began = resolve; });
    PM.MediaStore.put = async (...args: any[]) => { began(); await gate2; return realPut(...args); };
    const stopped = request('replace_media', { assetId: id, path: source });
    await started2; await request('__finish_run', { commit: false }); release();
    const stopError = await fail(stopped);
    PM.MediaStore.put = realPut;
    const afterStop = unchanged();
    const retry = await request('replace_media', { assetId: id, path: source }, 'media-new-run');
    return { fingerprintError, storageError, conflictError, stopError, afterStorage, afterConflict, afterStop, retry: JSON.parse(retry.content[0].text) };
  }, fixturePath('tone.mp3'));
  expect(result).toMatchObject({ afterStorage: true, afterConflict: true, afterStop: true, retry: { status: 'replaced', persisted: true } });
  expect(result.fingerprintError).toContain('expectedFingerprint');
  expect(result.storageError).toContain('store');
  expect(result.conflictError).toContain('changed');
  expect(result.stopError).toContain('changed');
  expect(session.diagnostics.pageErrors).toEqual([]);
});
