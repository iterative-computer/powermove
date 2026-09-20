import path from 'node:path';
import { readdir, stat } from 'node:fs/promises';
import { expect, test } from './helpers/app';

test('every export asks for its destination before rendering or showing progress', async ({ session }) => {
  await session.openEditor();
  await session.page.evaluate(() => {
    const PM = (window as any).PM;
    PM.pause();
    PM.proj = PM.mkProject({ name: 'Destination', w: 64, h: 64, fps: 2, dur: 1 });
    (window as any).__exportFrames = 0;
    const original = PM.renderFrameTo;
    PM.renderFrameTo = (...args: any[]) => { (window as any).__exportFrames++; return original(...args); };
  });
  await session.app.evaluate(({ dialog, BrowserWindow }) => {
    (globalThis as any).__exportPrompts = [];
    const capture = async () => {
      const state = await BrowserWindow.getAllWindows()[0]!.webContents.executeJavaScript(`({
        frames: window.__exportFrames,
        progress: !!document.querySelector('.export-progress'),
        busy: window.PM.Export.busy,
      })`);
      (globalThis as any).__exportPrompts.push(state);
    };
    dialog.showSaveDialog = async () => { await capture(); return { canceled: true, filePath: '' }; };
    dialog.showOpenDialog = async () => { await capture(); return { canceled: true, filePaths: [] }; };
  });
  for (const format of ['mp4', 'prores', 'webm', 'rec', 'png', 'still', 'web', 'json']) {
    expect(await session.page.evaluate(format => (window as any).PM.Export.run({ format, audio: false }), format)).toEqual({ cancelled: true });
    expect(await session.page.evaluate(() => (window as any).PM.Export.busy)).toBe(false);
  }
  expect(await session.app.evaluate(() => (globalThis as any).__exportPrompts)).toEqual(
    Array.from({ length: 8 }, () => ({ frames: 0, progress: false, busy: true })),
  );
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('still and sequence exports use the selected file or folder without another prompt', async ({ session }) => {
  await session.openEditor();
  await session.page.evaluate(() => {
    const PM = (window as any).PM;
    PM.pause();
    PM.proj = PM.mkProject({ name: 'Destination', w: 64, h: 64, fps: 2, dur: 1 });
  });
  const output = path.join(session.userData, 'chosen.png');
  await session.app.evaluate(({ dialog }, { output, folder }) => {
    (globalThis as any).__exportPromptCount = 0;
    dialog.showSaveDialog = async () => { (globalThis as any).__exportPromptCount++; return { canceled: false, filePath: output }; };
    dialog.showOpenDialog = async () => { (globalThis as any).__exportPromptCount++; return { canceled: false, filePaths: [folder] }; };
  }, { output, folder: session.userData });
  for (const format of ['still', 'png']) {
    expect(await session.page.evaluate(format => (window as any).PM.Export.run({ format, audio: false }), format)).toEqual({ cancelled: false });
  }
  expect((await stat(output)).size).toBeGreaterThan(0);
  expect(await readdir(session.userData)).toEqual(expect.arrayContaining(['Destination_00000.png', 'Destination_00001.png']));
  expect(await session.app.evaluate(() => (globalThis as any).__exportPromptCount)).toBe(2);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
