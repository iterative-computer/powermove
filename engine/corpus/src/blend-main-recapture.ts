/**
 * BLEND-1: attribute old corpus references to the pre-2b1fa379a blend
 * equation, then recapture only proven comparison kinds from today's app.
 *
 * Prove (starts Electron; run under pm-electron-lock):
 *   bun engine/corpus/src/blend-main-recapture.ts --prove /external/plan.json
 * Publish (metadata only, after reviewing the proof; no app launch):
 *   bun engine/corpus/src/blend-main-recapture.ts --publish /external/plan.proof.json
 *
 * The plan is the comparison plan produced from the failed corpus gate. This
 * tool neither chooses failures nor weakens a comparison. All inputs and
 * outputs must remain outside the public repository.
 */
import { execFileSync } from 'node:child_process';
import { existsSync, readFileSync, readdirSync, realpathSync, renameSync, unlinkSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { isDeepStrictEqual } from 'node:util';
import { launchLegacyApp } from './app';
import { desktopRoot, repoRoot } from './catalog';
import { sha256, timeLabel, writeFile, writeFloat, writePng, readFloat, type FileRecord } from './output';
import { installCorpusPage } from './page';
import { decodePng } from './png';

const ID = 'BLEND-1';
const REASON = 'recaptured after main 2b1fa379a blend change';
const OLD_RES = 'vec3 res = mix(dc, bc, s.a);';
const OLD_OUT = 'o = vec4(res * a, a);';
const CURRENT_RES = 'vec3 res = (1. - s.a) * d.rgb + (1. - d.a) * s.rgb + s.a * d.a * bc;';
const CURRENT_OUT = 'o = vec4(res, a);';
const PLAN_SCHEMA = 'powermove-colordepth-blend1-comparison-plan/v1';
const PROOF_SCHEMA = 'powermove-blend-main-recapture-proof/v1';

type Kind = 'opaque' | 'float';
type PlanItem = {
  kind: Kind; projectId: string; time: number; width: number; height: number;
  captureRoot: string; projectDir: string;
  manifest: { path: string; sha256: string };
  projectRecord: { path: string; sha256: string; id: string };
  loadedDocument: { path: string; sha256: string; record: FileRecord };
  original: { path: string; record: FileRecord; verifiedSha256: string };
  media: Record<string, { assetId: string; kind: string; source: { origin: string; path: string; bytes: number; sha256: string } }>;
};
type Plan = { schema: string; comparisons: PlanItem[]; comparisonCount: number };
type CaptureResult = {
  opaque: { options: Record<string, unknown>; base64: string };
  alpha?: { options: Record<string, unknown>; floatBase64: string };
  glErrors: string[]; warnings: string[]; prepareError?: string;
};
type Proven = {
  item: PlanItem; original: FileRecord | { data: FileRecord; header: FileRecord };
  current: FileRecord | { data: FileRecord; header: FileRecord };
  historical: FileRecord; evidence: FileRecord; oldCaptureSha256: string; currentCaptureSha256: string;
};
type Proof = {
  schema: string; reason: string; planFile: string; planSha256: string;
  mergedMainHead: string; appBuild: { sha256: string; files: number };
  equation: { old: [string, string]; current: [string, string] };
  setting: { useNewEngine: false; colorDepth: 16 };
  entries: Proven[];
};

function outsideRepo(file: string): string {
  const resolved = path.resolve(file);
  if (resolved === repoRoot || resolved.startsWith(repoRoot + path.sep)) throw new Error(`BLEND-1 input/output is inside the public repository: ${resolved}`);
  return resolved;
}

function verifiedBytes(file: string, hash: string, bytes?: number): Buffer {
  const data = readFileSync(file);
  if (sha256(data) !== hash || (bytes !== undefined && data.length !== bytes)) throw new Error(`Hash/size changed: ${file}`);
  return data;
}

function inRoot(root: string, relative: string): string {
  if (!relative || path.isAbsolute(relative)) throw new Error(`Expected a capture-relative path: ${relative}`);
  const full = path.resolve(root, relative);
  if (!full.startsWith(root + path.sep)) throw new Error(`Path escapes capture: ${relative}`);
  return full;
}

function buildFingerprint(): { sha256: string; files: number } {
  const root = path.join(desktopRoot, 'out');
  const files: string[] = [];
  const walk = (dir: string) => {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      const file = path.join(dir, entry.name);
      if (entry.isDirectory()) walk(file);
      else if (entry.isFile()) files.push(file);
    }
  };
  walk(root);
  if (!files.length) throw new Error('No built desktop app files');
  const hash = new Bun.CryptoHasher('sha256');
  for (const file of files.sort()) { hash.update(path.relative(root, file)); hash.update(readFileSync(file)); }
  return { sha256: hash.digest('hex'), files: files.length };
}

function head(): string {
  const result = execFileSync('git', ['rev-parse', 'HEAD'], { cwd: repoRoot, encoding: 'utf8' }).trim();
  execFileSync('git', ['merge-base', '--is-ancestor', '2b1fa379a', result], { cwd: repoRoot });
  return result;
}

function readPlan(file: string): { plan: Plan; bytes: Buffer } {
  const bytes = readFileSync(outsideRepo(realpathSync(file)));
  const plan = JSON.parse(bytes.toString()) as Plan;
  if (plan.schema !== PLAN_SCHEMA || !Array.isArray(plan.comparisons) || !plan.comparisons.length || plan.comparisonCount !== plan.comparisons.length) throw new Error('Wrong or empty BLEND-1 comparison plan');
  const seen = new Set<string>();
  for (const item of plan.comparisons) {
    if (!['opaque', 'float'].includes(item.kind) || !Number.isFinite(item.time) || !Number.isInteger(item.width) || !Number.isInteger(item.height) || item.width <= 0 || item.height <= 0) throw new Error('Malformed comparison');
    const key = `${item.captureRoot}\n${item.projectId}\n${item.time}\n${item.kind}`;
    if (seen.has(key)) throw new Error(`Duplicate comparison ${key}`);
    seen.add(key);
  }
  return { plan, bytes };
}

/** Recheck the plan against the saved capture, before any app or metadata work. */
function source(item: PlanItem) {
  const root = outsideRepo(realpathSync(item.captureRoot));
  const projectDir = realpathSync(item.projectDir);
  const relativeProjectDir = path.relative(root, projectDir);
  if (projectDir !== inRoot(root, relativeProjectDir) || projectDir !== inRoot(root, `projects/${item.projectId}`)) throw new Error(`Project directory does not match ${item.projectId}`);
  if (path.resolve(item.manifest.path) !== path.join(root, 'manifest.json') || path.resolve(item.projectRecord.path) !== path.join(projectDir, 'project.json') || path.resolve(item.loadedDocument.path) !== path.join(projectDir, 'loaded.json')) throw new Error('Plan paths do not match capture layout');
  const manifestBytes = verifiedBytes(item.manifest.path, item.manifest.sha256);
  const projectBytes = verifiedBytes(item.projectRecord.path, item.projectRecord.sha256);
  const loadedBytes = verifiedBytes(item.loadedDocument.path, item.loadedDocument.sha256, item.loadedDocument.record.bytes);
  if (item.loadedDocument.record.file !== path.relative(root, item.loadedDocument.path)) throw new Error('Loaded-document record path differs');
  const manifest = JSON.parse(manifestBytes.toString());
  const project = JSON.parse(projectBytes.toString());
  const loaded = JSON.parse(loadedBytes.toString());
  if (project.id !== item.projectId || item.projectRecord.id !== item.projectId || !loaded.id || (loaded.colorDepth !== undefined && loaded.colorDepth !== 16)) throw new Error('Project id or depth mismatch');
  const listed = manifest.projects?.find((p: any) => p.id === item.projectId);
  if (!listed || listed.dir !== relativeProjectDir) throw new Error(`Project absent from manifest: ${item.projectId}`);
  const local = project.samples?.find((s: any) => s.time === item.time);
  const global = listed.samples?.find((s: any) => s.time === item.time);
  if (!local || !global || !isDeepStrictEqual(local, global) || local.width !== item.width || local.height !== item.height || local.error || global.error) throw new Error(`Sample differs between project and manifest: ${item.projectId} @ ${item.time}`);
  if (local.blendMainRecapture?.[item.kind]) throw new Error(`Already published: ${item.projectId} @ ${item.time} ${item.kind}`);
  const original: FileRecord | { data: FileRecord; header: FileRecord } = item.kind === 'opaque' ? local.png : local.float;
  const chosen: FileRecord = item.kind === 'opaque' ? original as FileRecord : (original as { data: FileRecord }).data;
  if (!chosen || !isDeepStrictEqual(chosen, item.original.record) || item.original.verifiedSha256 !== chosen.sha256 || path.resolve(item.original.path) !== inRoot(root, chosen.file)) throw new Error(`Original record differs: ${item.projectId} @ ${item.time} ${item.kind}`);
  verifiedBytes(item.original.path, chosen.sha256, chosen.bytes);
  if (item.kind === 'float') {
    const float = original as { data: FileRecord; header: FileRecord };
    verifiedBytes(inRoot(root, float.header.file), float.header.sha256, float.header.bytes);
    const read = readFloat(inRoot(root, float.header.file));
    if (read.header.width !== item.width || read.header.height !== item.height || read.header.time !== item.time || read.header.sourcePrecision !== 'binary16') throw new Error('Original float header differs from depth-16 sample');
  }
  if (!isDeepStrictEqual(item.media, project.media)) throw new Error(`Plan media differs: ${item.projectId}`);
  const media = Object.entries(item.media).map(([name, entry]) => {
    if (!entry?.assetId || !entry.source || entry.source.origin === 'missing') throw new Error(`Unresolved original media: ${name}`);
    const file = entry.source.origin === 'fixture' ? path.join(repoRoot, entry.source.path) : path.resolve(root, entry.source.path);
    verifiedBytes(file, entry.source.sha256, entry.source.bytes);
    return { name, id: entry.assetId, kind: entry.kind, file, sha256: entry.source.sha256 };
  });
  return { root, relativeProjectDir, manifest, project, loaded, loadedBytes, sample: local, original, media };
}

/** Runs in the renderer. Change only the two FRAG_COMPOSITE expressions. */
function equation(mode: 'old' | 'current') {
  const PM = (window as any).PM;
  const oldRes = 'vec3 res = mix(dc, bc, s.a);';
  const oldOut = 'o = vec4(res * a, a);';
  const newRes = 'vec3 res = (1. - s.a) * d.rgb + (1. - d.a) * s.rgb + s.a * d.a * bc;';
  const newOut = 'o = vec4(res, a);';
  if (PM.store?.get?.('useNewEngine', false) !== false || PM.GL?.colorDepth !== 16) throw new Error('BLEND-1 requires Use New Engine OFF and legacy GL depth 16');
  const from: [string, string] = mode === 'old' ? [newRes, newOut] : [oldRes, oldOut];
  const to: [string, string] = mode === 'old' ? [oldRes, oldOut] : [newRes, newOut];
  const shader: string = PM.FRAG_COMPOSITE;
  if (mode === 'current' && shader.includes(newRes) && shader.includes(newOut) && !shader.includes(oldRes) && !shader.includes(oldOut)) return { useNewEngine: false, colorDepth: PM.GL.colorDepth, mode };
  if (from.some(s => shader.split(s).length !== 2) || to.some(s => shader.includes(s))) throw new Error(`Unexpected FRAG_COMPOSITE while switching to ${mode}`);
  PM.FRAG_COMPOSITE = shader.replace(from[0], to[0]).replace(from[1], to[1]);
  PM.GL.dropProgram('comp');
  PM.GL.dropProgram('adjustment');
  return { useNewEngine: false, colorDepth: PM.GL.colorDepth, mode };
}

async function capture(app: Awaited<ReturnType<typeof launchLegacyApp>>, item: PlanItem): Promise<Buffer> {
  const result: CaptureResult = await app.evaluate(args => (window as any).__corpus.capture(args), {
    time: item.time, width: item.width, height: item.height, alpha: item.kind === 'float', evaluate: false,
  });
  if (result.prepareError || result.glErrors.length || result.warnings.some(w => /GL error/i.test(w))) throw new Error(`Capture errors at ${item.projectId} @ ${item.time}: ${JSON.stringify({ prepareError: result.prepareError, glErrors: result.glErrors, warnings: result.warnings })}`);
  const base64 = item.kind === 'opaque' ? result.opaque?.base64 : result.alpha?.floatBase64;
  if (!base64) throw new Error('Capture returned no selected comparison kind');
  const bytes = Buffer.from(base64, 'base64');
  if (bytes.length !== item.width * item.height * (item.kind === 'float' ? 16 : 4)) throw new Error('Capture byte length differs from sample dimensions');
  return bytes;
}

async function prove(planFile: string) {
  const { plan, bytes: planBytes } = readPlan(planFile);
  const sources = plan.comparisons.map(source);
  const git = head(), appBuild = buildFingerprint();
  const pending: Array<{ item: PlanItem; original: Proven['original']; old: Buffer; current: Buffer; loadedHash: string; media: unknown }> = [];
  const failures: Array<{ projectId: string; time: number; kind: Kind; error: string }> = [];
  const groups = new Map<string, number[]>();
  plan.comparisons.forEach((item, index) => {
    const key = `${item.captureRoot}\n${item.projectId}`;
    groups.set(key, [...(groups.get(key) ?? []), index]);
  });
  for (const indexes of groups.values()) {
    const first = indexes[0];
    if (first === undefined) throw new Error('Empty comparison group');
    const item = plan.comparisons[first]!, input = sources[first]!;
    const app = await launchLegacyApp({ extensions: input.project.extensions ?? [] });
    try {
      await app.evaluate(installCorpusPage);
      await app.evaluate(doc => (window as any).__corpus.open(doc), input.loaded);
      if (input.media.length) {
        await app.evaluate(() => (window as any).__corpus.mediaInput());
        const ids: Record<string, string> = {};
        for (const medium of input.media) {
          await app.setInputFiles('#corpus-media', [medium.file]);
          const imported: Record<string, { id: string; kind: string; error?: string }> = await app.evaluate(() => (window as any).__corpus.importMedia());
          const found = imported[path.basename(medium.file)];
          if (!found?.id || found.error || found.kind !== medium.kind) throw new Error(`Media import differs: ${medium.name}`);
          ids[medium.id] = found.id;
        }
        const importedAssets: Record<string, unknown> = await app.evaluate(() => JSON.parse(JSON.stringify((window as any).PM.proj.assets)));
        const visit = (value: any): any => {
          if (typeof value === 'string') return ids[value] ?? value;
          if (Array.isArray(value)) return value.map(visit);
          if (value && typeof value === 'object') return Object.fromEntries(Object.entries(value).map(([key, child]) => [key, visit(child)]));
          return value;
        };
        const rewritten = visit(input.loaded);
        rewritten.assets = { ...input.loaded.assets };
        for (const [before, after] of Object.entries(ids)) {
          delete rewritten.assets[before];
          if (!importedAssets[after]) throw new Error(`Imported asset missing: ${before}`);
          rewritten.assets[after] = importedAssets[after];
        }
        await app.evaluate(doc => (window as any).__corpus.open(doc), rewritten);
        const pendingMedia: string[] = await app.evaluate(() => (window as any).__corpus.waitForMedia());
        if (pendingMedia.length) throw new Error(`Pending media: ${pendingMedia.join('; ')}`);
      }
      const setting = await app.evaluate(() => {
        const PM = (window as any).PM;
        return { useNewEngine: PM.store?.get?.('useNewEngine', false), colorDepth: PM.GL?.colorDepth, projectDepth: PM.proj?.colorDepth };
      });
      if (setting.useNewEngine !== false || setting.colorDepth !== 16 || (setting.projectDepth !== undefined && setting.projectDepth !== 16)) throw new Error(`Wrong legacy settings: ${JSON.stringify(setting)}`);
      for (const index of indexes) {
        const selected = plan.comparisons[index]!, saved = sources[index]!;
        try {
          if (selected.loadedDocument.sha256 !== sha256(input.loadedBytes) || !isDeepStrictEqual(selected.media, item.media)) throw new Error('Grouped samples have different document/media');
          let old: Buffer;
          try {
            await app.evaluate(equation, 'old');
            old = await capture(app, selected);
          } finally {
            // A failed old capture must never leave the app's shader mutated.
            await app.evaluate(equation, 'current');
          }
          const currentA = await capture(app, selected);
          const currentB = await capture(app, selected);
          if (!currentA.equals(currentB)) throw new Error(`Current capture is unstable: ${selected.projectId} @ ${selected.time} ${selected.kind}`);
          const baseline = selected.kind === 'opaque'
            ? Buffer.from(decodePng(readFileSync(selected.original.path)).data)
            : readFileSync(selected.original.path);
          if (!old.equals(baseline)) throw new Error(`Old-equation capture is not bit-exact with the original: ${selected.projectId} @ ${selected.time} ${selected.kind}; old=${sha256(old)} originalPixels=${sha256(baseline)}`);
          if (currentA.equals(old)) throw new Error(`Equation change does not explain this comparison: ${selected.projectId} @ ${selected.time} ${selected.kind}`);
          pending.push({ item: selected, original: saved.original, old, current: currentA, loadedHash: sha256(saved.loadedBytes), media: saved.media });
          console.log(`${ID} proved ${pending.length}/${plan.comparisons.length}: ${selected.projectId} t=${selected.time} ${selected.kind}; old exact, current repeated exact`);
        } catch (error) {
          failures.push({ projectId: selected.projectId, time: selected.time, kind: selected.kind, error: String((error as Error).message || error) });
          console.error(`${ID} unexplained ${selected.projectId} t=${selected.time} ${selected.kind}: ${String((error as Error).message || error)}`);
        }
      }
      if (app.pageErrors.length) throw new Error(`App page errors: ${app.pageErrors.join('; ')}`);
    } finally {
      try {
        await app.evaluate(() => {
          const PM = (window as any).PM;
          const oldRes = 'vec3 res = mix(dc, bc, s.a);', oldOut = 'o = vec4(res * a, a);';
          const newRes = 'vec3 res = (1. - s.a) * d.rgb + (1. - d.a) * s.rgb + s.a * d.a * bc;', newOut = 'o = vec4(res, a);';
          if (PM.FRAG_COMPOSITE.includes(oldRes) && PM.FRAG_COMPOSITE.includes(oldOut)) {
            PM.FRAG_COMPOSITE = PM.FRAG_COMPOSITE.replace(oldRes, newRes).replace(oldOut, newOut);
            PM.GL.dropProgram('comp'); PM.GL.dropProgram('adjustment');
          }
        });
      } finally { await app.close(); }
    }
  }
  if (failures.length) {
    const failureFile = outsideRepo(path.resolve(planFile.replace(/\.json$/, '') + '.proof-failures.json'));
    writeFileSync(failureFile, JSON.stringify({ schema: PROOF_SCHEMA, reason: REASON, attempted: plan.comparisons.length, proved: pending.length, failures }, null, 2) + '\n');
    throw new Error(`${ID}: ${failures.length}/${plan.comparisons.length} comparison kinds remain unexplained; no references or metadata written. See ${failureFile}`);
  }
  // Reference files are staged only after every selected kind proves exact.
  const created: string[] = [];
  let completed = false;
  try {
    const entries: Proven[] = [];
    for (const item of pending) {
      const root = outsideRepo(realpathSync(item.item.captureRoot));
      const rel = `${path.relative(root, item.item.projectDir)}/references/${ID}/${timeLabel(item.item.time)}.${item.item.kind}`;
      const oldRelative = rel + (item.item.kind === 'opaque' ? '.old.rgba8' : '.old.rgba32f');
      const extensions = item.item.kind === 'opaque' ? [rel + '.png', oldRelative, rel + '.evidence.json'] : [rel + '.rgba32f', rel + '.rgba32f.json', oldRelative, rel + '.evidence.json'];
      for (const file of extensions) if (existsSync(inRoot(root, file))) throw new Error(`BLEND-1 output already exists: ${file}`);
      const historical = writeFile(root, oldRelative, item.old);
      created.push(inRoot(root, historical.file));
      let current: Proven['current'];
      if (item.item.kind === 'opaque') {
        current = writePng(root, rel + '.png', item.item.width, item.item.height, item.current, {
          Software: 'Powermove legacy renderer', Source: `${ID}: main source-over equation`, Decision: REASON,
        });
        created.push(inRoot(root, current.file));
      } else {
        current = writeFloat(root, rel + '.rgba32f', {
          width: item.item.width, height: item.item.height, rowOrder: 'top-down', alpha: 'premultiplied', time: item.item.time,
          sourcePrecision: 'binary16', encoding: 'Legacy compositor working values: display encoding E, Rec.709 primaries, premultiplied, unclamped',
          source: `PM.GL.renderToPixels(t, w, h, {transparent:true,mblur:true,mbSamples:20}) with ${REASON}`,
        }, item.current);
        created.push(inRoot(root, current.data.file), inRoot(root, current.header.file));
      }
      const evidence = writeFile(root, rel + '.evidence.json', JSON.stringify({
        id: ID, reason: REASON, kind: item.item.kind, projectId: item.item.projectId, time: item.item.time,
        mainCommit: '2b1fa379a', original: item.original, historical, originalPixelSha256: sha256(item.old), oldCaptureSha256: sha256(item.old),
        current, currentCaptureSha256: sha256(item.current), oldExact: true, currentRepeatExact: true,
        equation: { old: [OLD_RES, OLD_OUT], current: [CURRENT_RES, CURRENT_OUT] },
        loadedDocumentSha256: item.loadedHash, media: item.media, mergedMainHead: git, appBuild,
        setting: { useNewEngine: false, colorDepth: 16 }, command: 'blend-main-recapture.ts --prove <external comparison plan>',
      }, null, 2) + '\n');
      created.push(inRoot(root, evidence.file));
      entries.push({ item: item.item, original: item.original, current, historical, evidence, oldCaptureSha256: sha256(item.old), currentCaptureSha256: sha256(item.current) });
    }
    const proof: Proof = { schema: PROOF_SCHEMA, reason: REASON, planFile: outsideRepo(realpathSync(planFile)), planSha256: sha256(planBytes), mergedMainHead: git, appBuild,
      equation: { old: [OLD_RES, OLD_OUT], current: [CURRENT_RES, CURRENT_OUT] }, setting: { useNewEngine: false, colorDepth: 16 }, entries };
    const proofFile = outsideRepo(path.resolve(planFile.replace(/\.json$/, '') + '.proof.json'));
    if (existsSync(proofFile)) throw new Error(`Proof already exists: ${proofFile}`);
    writeFileSync(proofFile, JSON.stringify(proof, null, 2) + '\n', { flag: 'wx' });
    completed = true;
    console.log(`${ID}: proved ${entries.length} comparison kinds; review ${proofFile} before --publish`);
  } finally {
    if (!completed) for (const file of created.reverse()) { try { unlinkSync(file); } catch { /* preserve the original failure */ } }
  }
}

function publish(proofFile: string) {
  const proof = JSON.parse(readFileSync(outsideRepo(realpathSync(proofFile)), 'utf8')) as Proof;
  if (proof.schema !== PROOF_SCHEMA || proof.reason !== REASON || !proof.entries?.length || !isDeepStrictEqual(proof.equation, { old: [OLD_RES, OLD_OUT], current: [CURRENT_RES, CURRENT_OUT] }) || !isDeepStrictEqual(proof.setting, { useNewEngine: false, colorDepth: 16 })) throw new Error('Invalid BLEND-1 proof');
  verifiedBytes(proof.planFile, proof.planSha256);
  const { plan } = readPlan(proof.planFile);
  if (plan.comparisons.length !== proof.entries.length || proof.entries.some((entry, index) => !isDeepStrictEqual(entry.item, plan.comparisons[index]))) throw new Error('Proof entries do not exactly match the comparison plan');
  if (head() !== proof.mergedMainHead || !isDeepStrictEqual(buildFingerprint(), proof.appBuild)) throw new Error('Merged head or app build changed since proof');
  const updates = new Map<string, { original: Buffer; next: Buffer }>();
  for (const entry of proof.entries) {
    const item = entry.item, saved = source(item), root = saved.root;
    const record = item.kind === 'opaque' ? entry.current as FileRecord : (entry.current as { data: FileRecord; header: FileRecord }).data;
    verifiedBytes(inRoot(root, record.file), record.sha256, record.bytes);
    if (item.kind === 'float') {
      const header = (entry.current as { data: FileRecord; header: FileRecord }).header;
      verifiedBytes(inRoot(root, header.file), header.sha256, header.bytes);
      readFloat(inRoot(root, header.file));
    }
    verifiedBytes(inRoot(root, entry.evidence.file), entry.evidence.sha256, entry.evidence.bytes);
    verifiedBytes(inRoot(root, entry.historical.file), entry.historical.sha256, entry.historical.bytes);
    if (entry.historical.sha256 !== entry.oldCaptureSha256) throw new Error('Historical capture hash differs from proof');
    const originalPixels = item.kind === 'opaque'
      ? Buffer.from(decodePng(readFileSync(item.original.path)).data)
      : readFileSync(item.original.path);
    if (sha256(originalPixels) !== entry.oldCaptureSha256) throw new Error('Historical capture no longer equals the original baseline');
    const currentPixels = item.kind === 'opaque'
      ? Buffer.from(decodePng(readFileSync(inRoot(root, record.file))).data)
      : readFileSync(inRoot(root, record.file));
    if (sha256(currentPixels) !== entry.currentCaptureSha256) throw new Error('Corrected reference differs from proven current capture');
    if (!isDeepStrictEqual(saved.original, entry.original)) throw new Error('Original record changed since proof');
    const evidence = JSON.parse(readFileSync(inRoot(root, entry.evidence.file), 'utf8'));
    if (!evidence.oldExact || !evidence.currentRepeatExact || evidence.mainCommit !== '2b1fa379a'
        || evidence.oldCaptureSha256 !== entry.oldCaptureSha256 || evidence.currentCaptureSha256 !== entry.currentCaptureSha256
        || !isDeepStrictEqual(evidence.original, entry.original) || !isDeepStrictEqual(evidence.current, entry.current)
        || !isDeepStrictEqual(evidence.historical, entry.historical) || !isDeepStrictEqual(evidence.setting, proof.setting)
        || !isDeepStrictEqual(evidence.equation, proof.equation) || evidence.mergedMainHead !== proof.mergedMainHead
        || !isDeepStrictEqual(evidence.appBuild, proof.appBuild)) throw new Error('Proof evidence changed');
    const manifestPath = item.manifest.path, projectPath = item.projectRecord.path;
    const manifest = updates.has(manifestPath) ? JSON.parse(updates.get(manifestPath)!.next.toString()) : saved.manifest;
    const project = updates.has(projectPath) ? JSON.parse(updates.get(projectPath)!.next.toString()) : saved.project;
    const manifestSample = manifest.projects.find((p: any) => p.id === item.projectId)?.samples.find((s: any) => s.time === item.time);
    const projectSample = project.samples.find((s: any) => s.time === item.time);
    if (!manifestSample || !projectSample) throw new Error('Sample disappeared during publish');
    for (const sample of [manifestSample, projectSample]) {
      if (sample.blendMainRecapture?.[item.kind]) throw new Error('Already published in metadata');
      sample.blendMainRecapture ??= {};
      sample.blendMainRecapture[item.kind] = { id: ID, mainCommit: '2b1fa379a', reason: REASON, original: entry.original,
        corrected: entry.current, historical: entry.historical, evidence: entry.evidence,
        oldCaptureSha256: entry.oldCaptureSha256, currentCaptureSha256: entry.currentCaptureSha256,
        mergedMainHead: proof.mergedMainHead, appBuildSha256: proof.appBuild.sha256 };
      if (item.kind === 'opaque') sample.png = entry.current;
      else sample.float = entry.current;
    }
    const oldBytes = item.kind === 'opaque' ? (entry.original as FileRecord).bytes : (entry.original as { data: FileRecord; header: FileRecord }).data.bytes + (entry.original as { data: FileRecord; header: FileRecord }).header.bytes;
    const newBytes = item.kind === 'opaque' ? (entry.current as FileRecord).bytes : (entry.current as { data: FileRecord; header: FileRecord }).data.bytes + (entry.current as { data: FileRecord; header: FileRecord }).header.bytes;
    manifest.totals.bytes += newBytes - oldBytes;
    updates.set(manifestPath, { original: verifiedBytes(manifestPath, item.manifest.sha256), next: Buffer.from(JSON.stringify(manifest, null, 1) + '\n') });
    updates.set(projectPath, { original: verifiedBytes(projectPath, item.projectRecord.sha256), next: Buffer.from(JSON.stringify(project, null, 1) + '\n') });
  }
  // Preflight every metadata hash before the first mutation. Atomic renames
  // then keep individual JSON files complete if another process reads them.
  for (const [file, value] of updates) if (sha256(readFileSync(file)) !== sha256(value.original)) throw new Error(`Concurrent metadata change: ${file}`);
  const written: string[] = [];
  try {
    for (const [file, value] of updates) {
      if (sha256(readFileSync(file)) !== sha256(value.original)) throw new Error(`Concurrent metadata change: ${file}`);
      const temp = `${file}.blend1-${process.pid}-${crypto.randomUUID()}.tmp`;
      try { writeFileSync(temp, value.next, { flag: 'wx' }); renameSync(temp, file); }
      finally { if (existsSync(temp)) unlinkSync(temp); }
      written.push(file);
    }
  } catch (error) {
    for (const file of written.reverse()) {
      const restore = `${file}.blend1-rollback-${process.pid}-${crypto.randomUUID()}.tmp`;
      writeFileSync(restore, updates.get(file)!.original, { flag: 'wx' }); renameSync(restore, file);
    }
    throw error;
  }
  console.log(`${ID}: published ${proof.entries.length} proven comparison kinds in ${updates.size} metadata files; original reference files remain untouched`);
}

/** Recheck immutable proof artifacts after publication, without expecting the
 * original project/manifest metadata hashes (which publication changes). */
function verifyProof(proofFile: string) {
  const proof = JSON.parse(readFileSync(outsideRepo(realpathSync(proofFile)), 'utf8')) as Proof;
  if (proof.schema !== PROOF_SCHEMA || proof.reason !== REASON || !isDeepStrictEqual(proof.setting, { useNewEngine: false, colorDepth: 16 })) throw new Error('Invalid BLEND-1 proof');
  verifiedBytes(proof.planFile, proof.planSha256);
  const { plan } = readPlan(proof.planFile);
  if (plan.comparisons.length !== proof.entries.length) throw new Error('Proof count differs from plan');
  for (const [index, entry] of proof.entries.entries()) {
    if (!isDeepStrictEqual(entry.item, plan.comparisons[index])) throw new Error(`Proof selection ${index} differs from plan`);
    const item = entry.item, root = outsideRepo(realpathSync(item.captureRoot));
    const historical = verifiedBytes(inRoot(root, entry.historical.file), entry.historical.sha256, entry.historical.bytes);
    const original = verifiedBytes(item.original.path, item.original.record.sha256, item.original.record.bytes);
    const originalPixels = item.kind === 'opaque' ? Buffer.from(decodePng(original).data) : original;
    if (!historical.equals(originalPixels) || sha256(historical) !== entry.oldCaptureSha256) throw new Error(`Historical proof differs from original: ${index}`);
    const record = item.kind === 'opaque' ? entry.current as FileRecord : (entry.current as { data: FileRecord; header: FileRecord }).data;
    const corrected = verifiedBytes(inRoot(root, record.file), record.sha256, record.bytes);
    const correctedPixels = item.kind === 'opaque' ? Buffer.from(decodePng(corrected).data) : corrected;
    if (sha256(correctedPixels) !== entry.currentCaptureSha256 || correctedPixels.equals(historical)) throw new Error(`Current proof differs or is unchanged: ${index}`);
    if (item.kind === 'float') {
      const header = (entry.current as { data: FileRecord; header: FileRecord }).header;
      verifiedBytes(inRoot(root, header.file), header.sha256, header.bytes);
      readFloat(inRoot(root, header.file));
    }
    const evidence = JSON.parse(verifiedBytes(inRoot(root, entry.evidence.file), entry.evidence.sha256, entry.evidence.bytes).toString());
    if (!evidence.oldExact || !evidence.currentRepeatExact || evidence.mainCommit !== '2b1fa379a'
        || !isDeepStrictEqual(evidence.original, entry.original) || !isDeepStrictEqual(evidence.current, entry.current)
        || !isDeepStrictEqual(evidence.historical, entry.historical) || !isDeepStrictEqual(evidence.setting, proof.setting)
        || !isDeepStrictEqual(evidence.equation, proof.equation) || evidence.mergedMainHead !== proof.mergedMainHead
        || !isDeepStrictEqual(evidence.appBuild, proof.appBuild)) throw new Error(`Evidence differs from proof: ${index}`);
  }
  console.log(`${ID}: verified ${proof.entries.length} immutable old/current captures and provenance; no app or metadata changed`);
}

if (import.meta.main) {
  const [mode, file] = process.argv.slice(2);
  if (!file || !mode || !['--check-plan', '--prove', '--publish', '--verify-proof'].includes(mode) || process.argv.length !== 4) throw new Error('Usage: blend-main-recapture.ts --check-plan|--prove <external-plan.json> | --publish|--verify-proof <external-proof.json>');
  if (mode === '--check-plan') {
    const { plan } = readPlan(file);
    for (const item of plan.comparisons) source(item);
    console.log(`${ID}: ${plan.comparisons.length} selected comparison kinds validated; no app or output opened`);
  } else if (mode === '--prove') await prove(file);
  else if (mode === '--verify-proof') verifyProof(file);
  else publish(file);
}
