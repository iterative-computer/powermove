/** Explicit owner-approved reference correction for only COMPCLOSE-1's two samples.
 * Run under pm-electron-lock: bun engine/corpus/src/compclose1.ts <motion capture>.
 * Original PNGs stay intact. No native renderer output is read by this tool.
 */
import { execFileSync } from 'node:child_process';
import { readdirSync, readFileSync, realpathSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { launchLegacyApp } from './app';
import { desktopRoot, repoRoot } from './catalog';
import { installCorpusPage } from './page';
import { sha256, timeLabel, writeFile, writePng } from './output';

const ID = 'COMPCLOSE-1', PROJECT = 'motion/blur-video';
const reason = 'Owner decision 2026-10-19: retain correct center-time video frames. The original legacy capture holds a stale upload; recapture the same legacy blur after invalidating source textures at the prepared center frame.';

/** Runs wholly in today's legacy renderer; only cached texture state is invalidated. */
async function correctedFrame(time: number) {
  const PM = (window as any).PM, gl = PM.GL.gl;
  const quality = PM.quality, busy = PM.Export.busy;
  const upload = gl.texImage2D;
  const uploads: any[] = [], centers: any[] = [], pendingGlErrors: number[] = [];
  const elements = new Map<HTMLVideoElement, string>();
  let phase = 'prime';
  const pixels = (data: Uint8Array) => {
    let s = ''; for (let i = 0; i < data.length; i += 0x8000) s += String.fromCharCode(...data.subarray(i, i + 0x8000));
    return btoa(s);
  };
  PM.pause(); PM.Export.busy = true; PM.quality = 1;
  try {
    await PM.prepareFrame(time);
    for (const layer of PM.proj.layers) {
      const asset = PM.assets.get(layer.d?.asset);
      if (layer.type !== 'video' || !asset?.el) continue;
      elements.set(asset.el, asset.id);
      const frame = new VideoFrame(asset.el);
      try {
        const fps = layer.name.includes('moving') ? 30 : 24;
        const expected = Math.round(Math.floor(time * fps) / fps * 1e6);
        if (Math.abs(frame.timestamp - expected) > 1) throw new Error(`Wrong center frame for ${layer.name}: ${frame.timestamp}, expected ${expected}`);
        centers.push({ layer: layer.name, asset: asset.id, currentTime: asset.el.currentTime, timestamp: frame.timestamp, duration: frame.duration });
      }
      finally { frame.close(); }
    }
    // Use the compositor's own invalidation and texFor upload. Its sample
    // transforms, shader, blend, shutter, count and source type are unchanged.
    PM.GL.dropTextures();
    for (let i = 0, error = gl.getError(); error && i < 32; i++, error = gl.getError()) pendingGlErrors.push(error);
    gl.texImage2D = function (...args: any[]) {
      const source = args.at(-1);
      if (source instanceof HTMLVideoElement) {
        const frame = new VideoFrame(source);
        try { uploads.push({ phase, asset: elements.get(source), timestamp: frame.timestamp, duration: frame.duration, currentTime: source.currentTime, width: frame.displayWidth, height: frame.displayHeight }); }
        finally { frame.close(); }
      }
      return upload.apply(this, args);
    };
    // Upload while each element is still at the prepared center. Seeking a
    // shutter sample can reduce readyState, so invalidation alone would skip
    // an uncached source. This sharp pass only primes texFor's source cache.
    const prepared = PM.preparedVideoFrames;
    const primingOptions = { mblur: false, opaque: true, topDownOpaque: true };
    PM.preparedVideoFrames = null;
    try { PM.GL.renderToPixels(time, 480, 270, primingOptions); }
    finally { PM.preparedVideoFrames = prepared; }
    phase = 'blur';
    const options = { mblur: true, mbSamples: 16, shutter: PM.proj.shutter || .5, opaque: true, topDownOpaque: true };
    const rgba = PM.GL.renderToPixels(time, 480, 270, options);
    const error = gl.getError();
    if (error) throw new Error(`GL error ${error}`);
    const expected = new Map(centers.filter(c => c.layer.includes('blur,')).map(c => [c.asset, c.timestamp]));
    if (new Set(uploads.map(c => c.asset)).size !== 2 || uploads.some(c => expected.get(c.asset) !== c.timestamp)) throw new Error(`Source uploads do not match both prepared center frames: ${JSON.stringify(uploads)}`);
    return { rgba: pixels(rgba), centers, uploads, primingOptions, options, pendingGlErrors };
  } finally {
    gl.texImage2D = upload; PM.preparedVideoFrames = null; PM.Export.busy = busy; PM.quality = quality;
  }
}

async function main() {
  const root = realpathSync(process.argv[2] || '');
  if (root === repoRoot || root.startsWith(repoRoot + path.sep)) throw new Error('Capture must stay outside the repository');
  const dir = path.join(root, 'projects', PROJECT), projectFile = path.join(dir, 'project.json');
  const record = JSON.parse(readFileSync(projectFile, 'utf8'));
  if (record.id !== PROJECT || record.samples.length !== 2 || record.samples.some((s: any, i: number) => s.time !== [.5, 1.2][i] || s.width !== 480 || s.height !== 270)) throw new Error('COMPCLOSE-1 scope is only motion/blur-video at 0.5 and 1.2, 480x270');
  const loadedBytes = readFileSync(path.join(dir, 'loaded.json'));
  const loaded = JSON.parse(loadedBytes.toString());
  const buildFiles: string[] = [], buildRoot = path.join(desktopRoot, 'out');
  const walk = (dir: string) => { for (const e of readdirSync(dir, { withFileTypes: true })) { const file = path.join(dir, e.name); if (e.isDirectory()) walk(file); else buildFiles.push(file); } };
  walk(buildRoot);
  const buildHash = new Bun.CryptoHasher('sha256');
  for (const file of buildFiles.sort()) { buildHash.update(path.relative(buildRoot, file)); buildHash.update(readFileSync(file)); }
  const appBuild = { sha256: buildHash.digest('hex'), files: buildFiles.length };
  const media = Object.entries<any>(record.media).map(([name, entry]) => {
    const file = entry.source.origin === 'fixture' ? path.join(repoRoot, entry.source.path) : path.resolve(root, entry.source.path);
    if (sha256(readFileSync(file)) !== entry.source.sha256) throw new Error(`Source changed: ${name}`);
    return { name, id: entry.assetId, file, sha256: entry.source.sha256 };
  });
  console.log(`${ID}: launching isolated hidden legacy app`);
  const app = await launchLegacyApp();
  try {
    await app.evaluate(installCorpusPage);
    await app.evaluate(doc => (window as any).__corpus.open(doc), loaded);
    await app.evaluate(() => (window as any).__corpus.mediaInput());
    await app.setInputFiles('#corpus-media', media.map(m => m.file));
    const imported: any = await app.evaluate(() => (window as any).__corpus.importMedia());
    const ids: Record<string, string> = Object.fromEntries(media.map(m => {
      if (typeof imported[m.name]?.id !== 'string' || !imported[m.name].id || imported[m.name].error) throw new Error(`Import failed: ${m.name}`);
      return [m.id, imported[m.name].id];
    }));
    const replaced = structuredClone(loaded);
    const importedAssets: any = await app.evaluate(() => JSON.parse(JSON.stringify((window as any).PM.proj.assets)));
    replaced.assets = Object.fromEntries(Object.values(ids).map(id => [id, importedAssets[id]]));
    for (const layer of replaced.layers) if (ids[layer.d?.asset]) layer.d.asset = ids[layer.d.asset];
    await app.evaluate(doc => (window as any).__corpus.open(doc), replaced);
    const pending = await app.evaluate(() => (window as any).__corpus.waitForMedia());
    if (pending.length) throw new Error(`Pending media: ${pending}`);
    console.log(`${ID}: original project and verified media loaded; capturing center uploads`);
    const versions = await app.evaluateMain(({ app }) => ({ app: app.getVersion(), ...process.versions }));
    for (const sample of record.samples) {
      const original = readFileSync(path.join(root, sample.png.file));
      if (sha256(original) !== sample.png.sha256) throw new Error('Original reference changed');
      const a = await app.evaluate(correctedFrame, sample.time);
      const b = await app.evaluate(correctedFrame, sample.time);
      if (a.rgba !== b.rgba) throw new Error(`Corrected capture is unstable at ${sample.time}`);
      const rel = `projects/${PROJECT}/references/${ID}/${timeLabel(sample.time)}`;
      const corrected = writePng(root, rel + '.png', 480, 270, new Uint8Array(Buffer.from(a.rgba, 'base64')), { Software: 'Powermove legacy renderer', Source: `${ID}: correct center upload, unchanged legacy 16-sample blur`, Decision: 'Owner 2026-10-19' });
      const evidence = writeFile(root, rel + '.evidence.json', JSON.stringify({ id: ID, ownerDecision: '2026-10-19', reason, project: PROJECT, time: sample.time, original: sample.png, corrected, sourceDocumentSha256: sha256(loadedBytes), media, versions, appBuild, git: execFileSync('git', ['rev-parse', 'HEAD'], { cwd: repoRoot, encoding: 'utf8' }).trim(), method: 'prepareFrame(t); GL.dropTextures(); sharp center-time render primes original texFor uploads; unchanged 16-sample GL.renderToPixels blur; texImage2D timestamp observer', ...a, rgba: undefined, repeatIdentical: true }, null, 2) + '\n');
      sample.referenceSubstitution = { id: ID, ownerDecision: '2026-10-19', reason, original: sample.png, corrected, evidence: [path.join(root, evidence.file)] };
      console.log(`${ID} ${PROJECT} t=${sample.time}: corrected legacy reference ${corrected.file}; upload PTS ${a.uploads.map(u => u.timestamp).join(',')}; repeat exact`);
    }
    if (app.pageErrors.length) throw new Error(`Page errors: ${app.pageErrors.join('; ')}`);
    // Publish metadata only after both independently repeated samples succeed.
    writeFileSync(projectFile, JSON.stringify(record, null, 1) + '\n');
    const manifestFile = path.join(root, 'manifest.json'), manifest = JSON.parse(readFileSync(manifestFile, 'utf8'));
    const item = manifest.projects.find((p: any) => p.id === PROJECT);
    if (!item) throw new Error('Capture manifest is missing motion/blur-video');
    item.samples = record.samples;
    writeFileSync(manifestFile, JSON.stringify(manifest, null, 1) + '\n');
  } finally { await app.close(); }
}
if (import.meta.main) await main();
