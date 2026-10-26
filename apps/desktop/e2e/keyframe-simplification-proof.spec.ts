import { copyFile, cp, mkdir, mkdtemp, readFile, rm, stat, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { expect, test, launchApp } from './helpers/app';

// Private inputs and output stay outside Git. Supply a prepared TEMPORARY copy
// containing project.json plus media.json [{id,file,mime,meta,sha256}].
test('every full-resolution frame has the same pixel hash on a copied project', async () => {
  test.skip(!process.env.PM_SIMPLIFY_PROOF_COPY, 'Opt-in private copied-project proof');
  test.setTimeout(1_800_000);
  const copy = await import('node:fs/promises').then(fs => fs.realpath(process.env.PM_SIMPLIFY_PROOF_COPY!));
  if (!copy.startsWith('/private/tmp/') && !copy.startsWith('/tmp/')) throw new Error('Proof input must be an isolated temporary copy');
  const media = JSON.parse(await readFile(path.join(copy, 'media.json'), 'utf8'));
  const requestedFrames = process.env.PM_SIMPLIFY_PROOF_FRAMES?.split(',').map(Number);
  const profile = await mkdtemp(path.join(tmpdir(), 'powermove-simplify-render-'));
  await mkdir(path.join(profile, 'store'));
  await copyFile(path.join(copy, 'project.json'), path.join(profile, 'store', 'project.simplifyProof.json'));
  if (await stat(path.join(copy, 'extensions')).catch(() => null))
    await cp(path.join(copy, 'extensions'), path.join(profile, 'extensions'), { recursive: true });
  const session = await launchApp({ userData: profile });
  try {
    await session.openEditor();
    for (const asset of media) {
      const bytes = await readFile(path.join(copy, asset.file));
      const { createHash } = await import('node:crypto');
      expect(createHash('sha256').update(bytes).digest('hex')).toBe(asset.sha256);
      expect(await session.page.evaluate(async ({ asset, base64 }) => {
        const PM = (window as any).PM;
        const binary = atob(base64), bytes = new Uint8Array(binary.length);
        for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
        return PM.MediaStore.put(asset.id, new Blob([bytes], { type: asset.mime }), asset.meta);
      }, { asset, base64: bytes.toString('base64') })).toBe(true);
    }
    const loaded = await session.page.evaluate(async (media) => {
      const PM = (window as any).PM;
      PM.pause(); PM.agentFrameCapture = true; PM.GL.cancelPreviewPreparation(); PM.AutomaticPreview?.cancel();
      PM.autosave = () => {};
      clearTimeout(PM.app.saveTimer);
      const raw = await PM.store.getAsync('project.simplifyProof', null);
      const source = typeof raw === 'string' ? JSON.parse(raw) : raw;
      const project = source.proj || source;
      // Clear original paths so recovery cannot reach outside the temp profile.
      for (const meta of Object.values(project.assets) as any[]) delete meta.sourcePath;
      window.dispatchEvent(new CustomEvent('pm-open-project', { detail: project }));
      await PM.restoreProjectAssets(PM.proj, false);
      PM.pause(); PM.agentFrameCapture = true; PM.perf.auto = false; PM.quality = 1;
      await Promise.all(project.layers.filter((l: any) => l.type === 'text').map((l: any) => PM.Fonts.ensure(l.d.font, l.d.weight)));
      await document.fonts.ready;
      const missing = media.filter((asset: any) => !PM.assets.get(asset.id)).map((asset: any) => asset.id);
      const missingEffects = PM.proj.layers.flatMap((layer: any) => (layer.fx || []).filter((fx: any) => !PM.FX[fx.type]).map((fx: any) => fx.type));
      return { missing, errors: [...PM.assets.errors.entries()], layers: project.layers.length,
        missingEffects, w: project.w, h: project.h, fps: project.fps, duration: project.dur, shaAvailable: !!crypto.subtle };
    }, media);
    console.log('SIMPLIFY_PROOF_LOADED', JSON.stringify(loaded));
    expect(loaded.missing).toEqual([]); expect(loaded.errors).toEqual([]); expect(loaded.missingEffects).toEqual([]); expect(loaded.shaAvailable).toBe(true);
    await session.page.exposeFunction('simplifyProofProgress', (value: any) => console.log('SIMPLIFY_PROOF_PROGRESS', JSON.stringify(value)));
    await session.page.exposeFunction('simplifyProofImage', async ({ name, data }: { name: string; data: string }) => {
      await writeFile(path.join(copy, name + '.png'), Buffer.from(data.split(',')[1], 'base64'));
    });
    const result = await session.page.evaluate(async (requestedFrames) => {
      const PM = (window as any).PM;
      const count = () => {
        let keys = 0;
        const visit = (value: any) => {
          if (!value || typeof value !== 'object') return;
          for (const [key, child] of Object.entries(value)) {
            if (key === 'kf' && Array.isArray(child)) keys += child.length;
            else if (key !== 'edits') visit(child);
          }
        };
        visit(PM.proj); return keys;
      };
      const bytes = () => new Blob([JSON.stringify({ v: PM.version, proj: PM.proj })]).size;
      const before = { keys: count(), bytes: bytes() };
      const frames = Math.ceil(PM.proj.dur * PM.proj.fps);
      const frameNumbers = requestedFrames || Array.from({ length: frames + 1 }, (_, i) => i);
      const hashes: string[][] = [];
      const timings: Record<string, number> = {};
      const capture = async (phase: string) => {
        const start = performance.now(), result: string[] = [];
        // Include the end boundary as well as all composition frames. Each
        // rendered frame uses the compositor's real 10 shutter samples.
        for (const frame of frameNumbers) {
          const time = frame / PM.proj.fps;
          await PM.prepareFrame(time);
          const pixels = PM.GL.renderToPixels(time, PM.proj.w, PM.proj.h,
            { opaque: true, exporting: true, previewReuse: false, occlusionCulling: false, mblur: true, mbSamples: 10, shutter: PM.proj.shutter || .5 });
          if (!pixels) throw new Error(`Frame ${frame} did not render`);
          if (PM.GL.errors.size) throw new Error(JSON.stringify([...PM.GL.errors.entries()]));
          if (requestedFrames) {
            const canvas = document.createElement('canvas'); canvas.width = PM.proj.w; canvas.height = PM.proj.h;
            canvas.getContext('2d')!.putImageData(new ImageData(new Uint8ClampedArray(pixels), canvas.width, canvas.height), 0, 0);
            await (window as any).simplifyProofImage({ name: `${phase}-${frame}`, data: canvas.toDataURL() });
          }
          const hash = await crypto.subtle.digest('SHA-256', pixels);
          result.push([...new Uint8Array(hash)].map(byte => byte.toString(16).padStart(2, '0')).join(''));
          if (frame % 60 === 0) await (window as any).simplifyProofProgress({ phase, frame, frames });
          await new Promise(resolve => setTimeout(resolve, 0));
        }
        timings[phase] = performance.now() - start; return result;
      };
      PM.rasterClear(); PM.touch();
      // Warm every layer span and custom program before recording the baseline.
      // Cold Canvas/driver startup must not stand in for completed artwork.
      await capture('warmup');
      hashes.push(await capture('before'));
      // Rebuild complete frames, retaining the unchanged glyph/media bitmaps
      // as the real edit command does. Re-rasterizing identical Canvas text
      // can vary individual RGB bytes independently of any project edit.
      PM.touch();
      const control = await capture('control');
      const controlMismatches = hashes[0].flatMap((hash, frame) => hash === control[frame] ? [] : [frameNumbers[frame]]);
      const start = performance.now();
      const simplified = PM.Edit.apply({ type: 'simplify_keyframes', scope: 'project' }, { origin: 'interface', label: 'Simplify Keyframes' });
      if (!simplified.ok) throw new Error(simplified.message);
      const simplifyMs = performance.now() - start;
      const after = { keys: count(), bytes: bytes() };
      PM.touch();
      hashes.push(await capture('after'));
      const mismatches = hashes[0].flatMap((hash, frame) => hash === hashes[1][frame] ? [] : [frameNumbers[frame]]);
      const serializedAfter = JSON.stringify({ v: PM.version, proj: PM.proj });
      const undo = { ok: PM.hist.undo(), keys: count() };
      const redo = { ok: PM.hist.redo(), keys: count() };
      return { before, after, simplifyMs, summary: simplified.data.results[0].data, framesCompared: hashes[0].length,
        mismatches, controlMismatches, hashes, timings, undo, redo, serializedAfter,
        renderer: { width: PM.proj.w, height: PM.proj.h, fps: PM.proj.fps, shutterSamples: 10 } };
    }, requestedFrames);
    const { serializedAfter, ...metrics } = result;
    await writeFile(path.join(copy, 'simplified.json'), serializedAfter);
    const savedFileBytes = (await stat(path.join(copy, 'simplified.json'))).size;
    const output = process.env.PM_SIMPLIFY_PROOF_REPORT || path.join(copy, 'proof-report.json');
    await writeFile(output, JSON.stringify({ ...metrics, loaded, savedFileBytes, sourceFileBytes: (await stat(path.join(copy, 'project.json'))).size }, null, 2));
    console.log('SIMPLIFY_PROOF_RESULT', JSON.stringify({ ...metrics, hashes: undefined, savedFileBytes }));
    expect(savedFileBytes).toBe(result.after.bytes);
    expect(result.undo).toEqual({ ok: true, keys: result.before.keys });
    expect(result.redo).toEqual({ ok: true, keys: result.after.keys });
    expect(result.after.keys).toBeLessThan(result.before.keys);
    // Keep the extra unchanged-source pass as a diagnostic. The existing
    // renderer can vary a few text RGB bytes between identical evaluations;
    // acceptance still requires every actual before/after hash to match.
    expect(result.mismatches).toEqual([]);
    expect(session.diagnostics.pageErrors).toEqual([]);
  } finally { await session.close(); await rm(profile, { recursive: true, force: true }); }
});
