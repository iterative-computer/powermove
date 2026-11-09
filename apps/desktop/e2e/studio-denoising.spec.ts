import path from 'node:path';
import { mkdir, writeFile } from 'node:fs/promises';
import { expect, test } from './helpers/app';

/** Opt-in capture of a local project. No project or images enter Git.
 * PM_DENOISE_PROJECT=/absolute/file.pmv PM_DENOISE_OUT=/tmp/...; PM_DENOISE_CLIP=1 exports only [16,20]. */
test('captures Studio denoising inputs and optionally the glass verification clip', async ({ session }) => {
  test.skip(!process.env.PM_DENOISE_PROJECT, 'Requires an explicit local verification project');
  test.setTimeout(7_200_000);
  const out = path.resolve(process.env.PM_DENOISE_OUT || '/tmp/pm-shots/studio3d/laneDN');
  await mkdir(out, { recursive: true });
  await session.openEditor();
  await session.app.evaluate(async ({ ipcMain, dialog }, { project, out }) => {
    const fs = process.getBuiltinModule('fs')!;
    dialog.showOpenDialog = async () => ({ canceled: false, filePaths: [project] });
    dialog.showMessageBox = async () => ({ response: 1, checkboxChecked: false });
    dialog.showSaveDialog = async () => ({ canceled: false, filePath: `${out}/glass-after.mp4` });
    const original = (ipcMain as any)._invokeHandlers.get('studio-native:request');

    const inputs = new Map<string, Buffer[]>();
    ipcMain.removeHandler('studio-native:request');
    ipcMain.handle('studio-native:request', async (event, key, command, binary) => {
      if (!inputs.has(key)) inputs.set(key, []);
      if (['prepared', 'preparedBegin', 'preparedChunk', 'preparedCommit', 'preparedPatch', 'preparedView', 'textures', 'scene', 'patch', 'screen'].includes(command.op)) {
        const json = Buffer.from(JSON.stringify({ id: 1, ...command }));
        const payload = Buffer.from(binary || []), header = Buffer.alloc(12);
        header.write('PMS1'); header.writeUInt32LE(json.length, 4); header.writeUInt32LE(payload.length, 8);
        inputs.get(key)!.push(Buffer.concat([header, json, payload]));
      }
      if (command.op === 'render' && command.tier === 'final' && (globalThis as any).__denoiseCapture) {
        fs.writeFileSync(`${out}/glass-${command.time.toFixed(6)}.pms`, Buffer.concat(inputs.get(key)!));
      }
      return original(event, key, command, binary);
    });
  }, { project: path.resolve(process.env.PM_DENOISE_PROJECT!), out });
  await session.page.evaluate(async () => { const PM = (window as any).PM; await PM.openProject(); PM.pause(); PM.AutomaticPreview?.cancel(); PM.Studio?.setPreview('fast'); });
  await session.app.evaluate(() => { (globalThis as any).__denoiseCapture = true; });
  for (const t of [16.5, 17.5, 19.5]) {
    const frame = await session.page.evaluate(async t => {
      const PM = (window as any).PM; PM.agentFrameCapture = true;
      try { await PM.prepareFrame(t, { exporting: true, width: 320, height: 180, mblur: false }); return { png: PM.renderFrameTo(t, 320, 180, { exporting: true, mblur: false }).toDataURL('image/png'), status: PM.Studio.status(), diagnostics: PM.Studio.diagnostics(), capture: PM.Studio.captureDiagnostics() }; }
      finally { PM.agentFrameCapture = false; }
    }, t);
    expect(frame.status.backend).toBe('native-studio');
    await writeFile(path.join(out, `project-${t}.json`), JSON.stringify(frame));
    await writeFile(path.join(out, `project-${t}.png`), Buffer.from(frame.png.split(',')[1]!, 'base64'));
  }
  await session.app.evaluate(() => { (globalThis as any).__denoiseCapture = false; });
  if (process.env.PM_DENOISE_CLIP) {
    const start = Date.now();
    const result = await session.page.evaluate(async () => {
      const PM = (window as any).PM; PM.proj.work = [16, 20];
      return PM.Export.run({ format: 'mp4', scale: 1, fps: 30, range: 'work', quality: 'max', audio: false, alpha: false, mblur: true, mbSamples: 2, reportPath: true });
    });
    await writeFile(path.join(out, 'glass-export.json'), JSON.stringify({ seconds: (Date.now() - start) / 1000, frames: 120, result }, null, 2));
    expect(result.error).toBeUndefined(); expect(result.cancelled).toBe(false);
  }
  expect(session.diagnostics.pageErrors).toEqual([]);
});
