import { execFileSync } from 'node:child_process';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import path from 'node:path';
import os from 'node:os';
import { decodeProjectContainer } from '../src/shared/project-container';
import { expect, test, repoRoot } from './helpers/app';

// Optional real-project acceptance run, using a copy and an isolated profile.
test('reopens a media project, preserves source, and exports advancing frames', async ({ session }) => {
  test.skip(!process.env.POWERMOVE_EXPORT_FIXTURE, 'Set POWERMOVE_EXPORT_FIXTURE to a project file');
  test.setTimeout(180_000);
  const root = await mkdtemp(path.join(os.tmpdir(), 'powermove-project-export-'));
  try {
    const fixture = process.env.POWERMOVE_EXPORT_FIXTURE!;
    const original = decodeProjectContainer(await readFile(fixture)).document.proj;
    await session.app.evaluate(({ dialog }, filePath) => {
      dialog.showOpenDialog = async () => ({ canceled: false, filePaths: [filePath] });
    }, fixture);
    await session.page.evaluate(() => (window as any).PM.openProject());
    await session.page.waitForFunction(() => {
      const PM = (window as any).PM;
      return PM.proj.layers.filter((l: any) => l.type === 'video').every((l: any) => PM.assets.get(l.d.asset)?.el?.readyState >= 2);
    });
    const loaded = await session.page.evaluate(() => {
      const PM = (window as any).PM;
      return { layers: PM.proj.layers, editsBytes: JSON.stringify(PM.proj.edits).length };
    });
    const sourceLayers = original.layers.map((layer: any) => ({ ...layer, solo: !!layer.solo }));
    expect(loaded.layers).toEqual(sourceLayers);
    expect(loaded.editsBytes).toBeLessThan(1_100_000);
    if (process.env.POWERMOVE_OPTIMIZE_SOURCE) {
      const measureSeeks = () => session.page.evaluate(async () => {
        const PM = (window as any).PM;
        PM.pause(); PM.Export.busy = true;
        const start = performance.now();
        try {
          for (const time of [4.8, 1.2, 4.2, 2.1, 3.8, 1.6, 4.6, 2.8]) await PM.prepareFrame(time);
          return performance.now() - start;
        } finally { PM.Export.busy = false; PM.preparedVideoFrames = null; }
      });
      const before = await measureSeeks();
      await session.page.evaluate(() => {
        const input = document.createElement('input'); input.type = 'file'; input.id = 'upgrade-source'; document.body.appendChild(input);
      });
      await session.page.locator('#upgrade-source').setInputFiles(process.env.POWERMOVE_OPTIMIZE_SOURCE);
      const version = await session.page.evaluate(async () => {
        const PM = (window as any).PM;
        const asset = await PM.assets.add((document.querySelector('#upgrade-source') as HTMLInputElement).files![0]);
        return asset.playbackProxyVersion;
      });
      expect(version).toBe(3);
      const after = await measureSeeks();
      console.log(JSON.stringify({ randomSeekMillisecondsBefore: before, randomSeekMillisecondsAfter: after }));
      expect(after).toBeLessThan(before);
    }
    const savedPath = path.join(root, 'Reopened.pmv');
    await session.app.evaluate(({ dialog }, filePath) => {
      dialog.showSaveDialog = async () => ({ canceled: false, filePath });
    }, savedPath);
    expect(await session.page.evaluate(() => (window as any).PM.saveProject({ saveAs: true }))).toBe(true);
    await session.relaunch();
    await session.page.waitForFunction(() => {
      const PM = (window as any).PM;
      return PM.proj.layers.length > 0 && PM.proj.layers.filter((l: any) => l.type === 'video').every((l: any) => PM.assets.get(l.d.asset)?.el?.readyState >= 2);
    });
    const result = await session.page.evaluate(async () => {
      const PM = (window as any).PM;
      let data = '';
      PM.download = async (blob: Blob) => {
        data = await new Promise<string>(resolve => {
          const reader = new FileReader(); reader.onload = () => resolve((reader.result as string).split(',')[1]!); reader.readAsDataURL(blob);
        });
      };
      PM.setTime(PM.proj.work[1] - 0.5, { force: true });
      const exported = await PM.Export.run({ ...PM.proj.exportDefaults, format: 'webm', scale: 1, range: 'work' });
      if (exported.error) throw new Error(exported.error);
      return { data, layers: PM.proj.layers, frames: Math.ceil((PM.proj.work[1] - PM.proj.work[0]) * PM.proj.fps) };
    });
    expect(result.layers).toEqual(sourceLayers);
    const output = process.env.POWERMOVE_EXPORT_OUTPUT || path.join(root, 'Export.webm');
    await writeFile(output, Buffer.from(result.data, 'base64'));
    const hashes = execFileSync(path.join(repoRoot, 'node_modules/ffmpeg-static/ffmpeg'), ['-v', 'error', '-i', output, '-an', '-f', 'framemd5', 'pipe:1'], { encoding: 'utf8' })
      .split('\n').filter(line => line && !line.startsWith('#')).map(line => line.split(',').at(-1)!.trim());
    expect(hashes.length).toBe(result.frames);
    console.log(JSON.stringify({ originalLogBytes: JSON.stringify(original.edits).length, compactedLogBytes: loaded.editsBytes, frames: hashes.length, uniqueFrames: new Set(hashes).size, output }));
  } finally { await rm(root, { recursive: true, force: true }); }
});
