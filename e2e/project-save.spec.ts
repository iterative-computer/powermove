import { readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { expect, test, type LaunchedApp } from './helpers/app';

async function saveTo(session: LaunchedApp, destination: string | null) {
  await session.app.evaluate(({ dialog }, filePath) => {
    dialog.showSaveDialog = async () => {
      (globalThis as any).__saveDialogs = ((globalThis as any).__saveDialogs || 0) + 1;
      return { canceled: !filePath, filePath: filePath || '' };
    };
  }, destination);
}
async function rename(session: LaunchedApp, name: string) {
  await session.page.evaluate(name => {
    const PM = (window as any).PM;
    PM.Projects.rename(PM.proj.id, name);
  }, name);
}
const savedName = async (file: string) => {
  try { return JSON.parse(await readFile(file, 'utf8')).proj.name; }
  catch (error: any) { if (error.code === 'ENOENT') return null; throw error; }
};

test('Command+S writes real files, reuses the destination, saves from a field, and Save As changes it', async ({ session }) => {
  const a = path.join(session.userData, 'First.pmv'), b = path.join(session.userData, 'Copy.pmv');
  await saveTo(session, a);
  await session.page.keyboard.press('Meta+S');
  await expect.poll(() => savedName(a)).toBe('Velocity Study');
  await expect.poll(() => session.page.evaluate(() => (window as any).PM.app.dirty)).toBe(false);
  await rename(session, 'Second version');
  await session.page.waitForTimeout(750);
  expect(await session.page.evaluate(() => (window as any).PM.app.dirty)).toBe(true);
  expect(await savedName(a)).toBe('Velocity Study');
  await session.page.evaluate(() => {
    const input = document.createElement('input'); input.id = 'save-from-field'; document.body.appendChild(input); input.focus();
  });
  await session.page.keyboard.press('Meta+S');
  await expect.poll(() => savedName(a)).toBe('Second version');
  expect(await savedName(a + '1')).toBe('Velocity Study');
  expect(await session.app.evaluate(() => (globalThis as any).__saveDialogs)).toBe(1);
  await saveTo(session, b);
  await session.page.keyboard.press('Meta+Shift+S');
  await expect.poll(() => savedName(b)).toBe('Second version');
  await expect.poll(() => session.page.evaluate(() => (window as any).PM.app.saving)).toBe(false);
  await session.relaunch();
  await expect.poll(() => session.page.evaluate(() => (window as any).PM.app.dirty)).toBe(false);
  await rename(session, 'After relaunch');
  await session.page.keyboard.press('Meta+S');
  await expect.poll(() => savedName(b)).toBe('After relaunch');
  expect(await savedName(a)).toBe('Second version');
  expect(await session.app.evaluate(() => (globalThis as any).__saveDialogs || 0)).toBe(0);
  await session.page.screenshot({ path: '/tmp/powermove-save-system-verified.png' });
});

test('native Open preserves editable source and subsequent saves update the opened file', async ({ session }) => {
  const destination = path.join(session.userData, 'Editable.pmv');
  await saveTo(session, destination);
  const oldId = await session.page.evaluate(() => (window as any).PM.proj.id);
  expect(await session.page.evaluate(() => (window as any).PM.saveProject())).toBe(true);
  await session.app.evaluate(({ dialog }, filePath) => {
    dialog.showOpenDialog = async () => ({ canceled: false, filePaths: [filePath] });
  }, destination);
  await session.page.evaluate(() => (window as any).PM.openProject());
  expect(await session.page.evaluate(() => (window as any).PM.proj.id)).not.toBe(oldId);
  await expect.poll(() => session.page.evaluate(() => (window as any).PM.app.dirty)).toBe(false);
  await rename(session, 'Opened and edited');
  expect(await session.page.evaluate(() => (window as any).PM.saveProject())).toBe(true);
  expect(await savedName(destination)).toBe('Opened and edited');
  expect(await session.app.evaluate(() => (globalThis as any).__saveDialogs)).toBe(1);
  const undone = await session.page.evaluate(() => {
    const PM = (window as any).PM, layer = PM.proj.layers[0], original = layer.name;
    PM.Edit.apply({ type: 'set_layer', target: layer.id, patch: { name: 'Editable layer' } });
    const changed = PM.proj.layers[0].name;
    PM.hist.undo();
    return { changed, original, restored: PM.proj.layers[0].name };
  });
  expect(undone.changed).toBe('Editable layer');
  expect(undone.restored).toBe(undone.original);
});

test('cancel, disk-write failures, and external modifications never clear unsaved changes', async ({ session }) => {
  const destination = path.join(session.userData, 'Safe.pmv');
  await rename(session, 'Unsaved work');
  await saveTo(session, null);
  expect(await session.page.evaluate(() => (window as any).PM.saveProject())).toBe(false);
  expect(await session.page.evaluate(() => (window as any).PM.app.dirty)).toBe(true);
  await saveTo(session, path.join(session.userData, 'missing-folder', 'bad.pmv'));
  expect(await session.page.evaluate(() => (window as any).PM.saveProject())).toBe(false);
  expect(await session.page.evaluate(() => (window as any).PM.app.dirty)).toBe(true);
  await saveTo(session, destination);
  expect(await session.page.evaluate(() => (window as any).PM.saveProject())).toBe(true);
  await writeFile(destination, 'external work');
  await rename(session, 'New edit');
  expect(await session.page.evaluate(() => (window as any).PM.saveProject())).toBe(false);
  expect(await readFile(destination, 'utf8')).toBe('external work');
  expect(await session.page.evaluate(() => (window as any).PM.app.dirty)).toBe(true);
});

test('closing a tab and the native window honors Cancel and cancelled Save', async ({ session }) => {
  await rename(session, 'Keep this open');
  await session.app.evaluate(({ dialog }) => {
    dialog.showMessageBox = async () => {
      (globalThis as any).__closePrompts = ((globalThis as any).__closePrompts || 0) + 1;
      return { response: 1, checkboxChecked: false };
    };
  });
  await session.page.getByRole('button', { name: 'Close Keep this open', exact: true }).click();
  await expect.poll(() => session.app.evaluate(() => (globalThis as any).__closePrompts)).toBe(1);
  expect(await session.page.evaluate(() => (window as any).PM.proj.name)).toBe('Keep this open');
  await session.app.evaluate(({ BrowserWindow }) => { BrowserWindow.getAllWindows()[0]!.close(); });
  await expect.poll(() => session.app.evaluate(() => (globalThis as any).__closePrompts)).toBe(2);
  expect(await session.app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows().length)).toBe(1);
  await session.app.evaluate(({ dialog }) => {
    dialog.showMessageBox = async () => ({ response: 0, checkboxChecked: false });
  });
  await saveTo(session, null);
  expect(await session.page.evaluate(() => (window as any).PM.prepareToClose())).toBe(false);
  // Cleanup is a harness operation, not a user close decision.
  await session.app.evaluate(({ dialog }) => { dialog.showMessageBox = async () => ({ response: 2, checkboxChecked: false }); });
});

test('Save on native window close writes the document before closing', async ({ session }) => {
  const destination = path.join(session.userData, 'On close.pmv');
  await rename(session, 'Saved on close');
  await saveTo(session, destination);
  await session.app.evaluate(({ dialog }) => {
    dialog.showMessageBox = async () => ({ response: 0, checkboxChecked: false });
  });
  await session.app.evaluate(({ BrowserWindow }) => { BrowserWindow.getAllWindows()[0]!.close(); });
  await expect.poll(() => session.app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows().length)).toBe(0);
  expect(await savedName(destination)).toBe('Saved on close');
});

test('native Quit honors Cancel without closing the editing session', async ({ session }) => {
  await rename(session, 'Cancel quitting');
  await session.app.evaluate(({ dialog, app }) => {
    dialog.showMessageBox = async () => {
      (globalThis as any).__quitPrompt = true;
      return { response: 1, checkboxChecked: false };
    };
    app.quit();
  });
  await expect.poll(() => session.app.evaluate(() => (globalThis as any).__quitPrompt)).toBe(true);
  expect(await session.page.evaluate(() => (window as any).PM.proj.name)).toBe('Cancel quitting');
  expect(await session.app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows().length)).toBe(1);
  await session.app.evaluate(({ dialog }) => { dialog.showMessageBox = async () => ({ response: 2, checkboxChecked: false }); });
});
