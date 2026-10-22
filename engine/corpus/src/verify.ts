/**
 * Verify a capture against its manifest.
 *
 *   bun engine/corpus/src/verify.ts <capture-dir>
 *
 * Every file the manifest lists must exist with its recorded size and SHA-256;
 * PNGs must decode at the sample size; float frames must match their headers;
 * scenario hashes must match their raw bytes; totals must agree with the
 * project list. Files in the capture the manifest does not account for are
 * reported. Exits 1 on any problem.
 */
import { existsSync, readdirSync, readFileSync, realpathSync, statSync } from 'node:fs';
import path from 'node:path';
import { isDeepStrictEqual } from 'node:util';
import { readFloat, sha256, type FileRecord } from './output';
import { decodePng } from './png';

export interface VerifyResult { problems: string[]; checked: number; unlisted: string[] }

const isRecord = (value: unknown): value is FileRecord =>
  !!value && typeof value === 'object' && typeof (value as FileRecord).file === 'string' && typeof (value as FileRecord).sha256 === 'string';

export function verifyCapture(dir: string): VerifyResult {
  const problems: string[] = [];
  const listed = new Set<string>(['manifest.json', 'capture.log']);
  let checked = 0;
  const manifestPath = path.join(dir, 'manifest.json');
  if (!existsSync(manifestPath)) return { problems: [`${manifestPath} is missing`], checked, unlisted: [] };
  const manifest = JSON.parse(readFileSync(manifestPath, 'utf8'));
  if (manifest.format !== 'powermove-engine-corpus' || manifest.version !== 1) problems.push(`unexpected manifest format ${manifest.format} v${manifest.version}`);

  const check = (record: FileRecord | undefined, what: string): Buffer | null => {
    if (!record) return null;
    if (!isRecord(record)) { problems.push(`${what}: malformed file record`); return null; }
    listed.add(record.file);
    const file = path.join(dir, record.file);
    if (!existsSync(file)) { problems.push(`${what}: ${record.file} is missing`); return null; }
    const bytes = readFileSync(file);
    checked++;
    if (bytes.length !== record.bytes) problems.push(`${what}: ${record.file} is ${bytes.length} bytes, manifest says ${record.bytes}`);
    if (sha256(bytes) !== record.sha256) problems.push(`${what}: ${record.file} does not match its SHA-256`);
    return bytes;
  };
  const checkPng = (record: FileRecord | undefined, what: string, width: number, height: number) => {
    const bytes = check(record, what);
    if (!bytes) return;
    try {
      const png = decodePng(new Uint8Array(bytes));
      if (png.width !== width || png.height !== height) problems.push(`${what}: ${record!.file} is ${png.width}x${png.height}, expected ${width}x${height}`);
    } catch (error) { problems.push(`${what}: ${record!.file} does not decode: ${(error as Error).message}`); }
  };

  let frames = 0, floatFrames = 0, evalDumps = 0, blenderDumps = 0, failedSamples = 0, bytes = 0;
  for (const project of manifest.projects ?? []) {
    const where = project.id;
    check(project.document, `${where} document`);
    check(project.loaded, `${where} loaded`);
    listed.add(`${project.dir}/project.json`);
    const projectPath = path.join(dir, project.dir, 'project.json');
    if (!existsSync(projectPath)) problems.push(`${where}: ${project.dir}/project.json is missing`);
    const localSamples = existsSync(projectPath) ? JSON.parse(readFileSync(projectPath, 'utf8')).samples ?? [] : [];
    for (const asset of project.blender?.assets ?? []) { bytes += asset.bytes; check(asset, `${where} Blender asset`); }
    for (const sample of project.samples ?? []) {
      const at = `${where} @ ${sample.time}`;
      if (sample.error) failedSamples++;
      if (sample.png) { frames++; bytes += sample.png.bytes; }
      checkPng(sample.png, `${at} png`, sample.width, sample.height);
      // Owner annotations add references alongside the immutable original
      // outputs; they do not rewrite the original capture's byte totals.
      const substitution = sample.referenceSubstitution;
      if (!isDeepStrictEqual(substitution, localSamples.find((s: any) => s.time === sample.time)?.referenceSubstitution)) problems.push(`${at}: reference substitution metadata differs between project and manifest`);
      if (substitution !== undefined) {
        if (substitution?.id !== 'COMPCLOSE-1' || substitution.ownerDecision !== '2026-10-19'
            || project.id !== 'motion/blur-video' || ![.5, 1.2].includes(sample.time)
            || sample.width !== 480 || sample.height !== 270 || typeof substitution.reason !== 'string' || !substitution.reason.trim()
            || !Array.isArray(substitution.evidence) || !substitution.evidence.length) {
          problems.push(`${at}: invalid COMPCLOSE-1 reference substitution scope`);
        } else {
          if (['file', 'bytes', 'sha256'].some(key => substitution.original?.[key] !== sample.png?.[key])) problems.push(`${at}: substitution original differs from original PNG record`);
          if (!substitution.corrected) problems.push(`${at}: corrected PNG is missing`);
          else checkPng(substitution.corrected, `${at} COMPCLOSE-1 corrected png`, sample.width, sample.height);
          for (const evidence of substitution.evidence) {
            if (typeof evidence !== 'string' || !path.isAbsolute(evidence) || !existsSync(evidence) || !statSync(evidence).isFile()) problems.push(`${at}: missing absolute COMPCLOSE-1 evidence file`);
            else {
              // Evidence travels with the capture: a temporary or other
              // outside path would vanish and break the gate later.
              const relative = path.relative(realpathSync(dir), realpathSync(evidence));
              if (!relative || relative.startsWith('..') || path.isAbsolute(relative)) problems.push(`${at}: COMPCLOSE-1 evidence is outside the capture: ${evidence}`);
              else listed.add(relative);
            }
          }
        }
      }
      if (sample.alphaPng) bytes += sample.alphaPng.bytes;
      checkPng(sample.alphaPng, `${at} alpha png`, sample.width, sample.height);
      const floats = [
        ...(sample.float ? [{ float: sample.float, at }] : []),
        ...((sample.blur ?? []) as Array<{ samples: number; float: any }>).map(b => ({ float: b.float, at: `${at} (${b.samples} blur samples)` })),
      ];
      for (const { float, at } of floats) {
        floatFrames++;
        bytes += float.data.bytes + float.header.bytes;
        const data = check(float.data, `${at} float data`), header = check(float.header, `${at} float header`);
        if (data && header) {
          try {
            const read = readFloat(path.join(dir, float.header.file));
            if (read.header.width !== sample.width || read.header.height !== sample.height) problems.push(`${at}: float header is ${read.header.width}x${read.header.height}`);
            if (read.header.time !== sample.time) problems.push(`${at}: float header time ${read.header.time}`);
            let nonFinite = 0;
            for (const value of read.pixels) if (!Number.isFinite(value)) nonFinite++;
            if (nonFinite) problems.push(`${at}: ${nonFinite} non-finite float values`);
          } catch (error) { problems.push(`${at}: ${(error as Error).message}`); }
        }
      }
      if (sample.blender) {
        blenderDumps++;
        bytes += sample.blender.bytes;
        const dump = check(sample.blender, `${at} blender`);
        if (dump) {
          try {
            const parsed = JSON.parse(dump.toString('utf8'));
            if (parsed.format !== 'powermove-corpus-blender' || parsed.time !== sample.time) problems.push(`${at}: Blender dump has format ${parsed.format} at ${parsed.time}`);
            for (const pass of [parsed.final, parsed.preview, parsed.export, parsed.exportAlpha]) for (const snapshot of pass?.snapshots ?? []) for (const asset of snapshot.assets ?? []) {
              if (!(project.blender?.assets ?? []).some((record: FileRecord) => record.file === asset.file && record.sha256 === asset.sha256)) problems.push(`${at}: Blender asset ${asset.file} is not in the project's asset list`);
            }
          } catch (error) { problems.push(`${at}: Blender dump is not JSON: ${(error as Error).message}`); }
        }
      }
      if (sample.eval) {
        evalDumps++;
        bytes += sample.eval.bytes;
        const dump = check(sample.eval, `${at} eval`);
        if (dump) {
          try {
            const parsed = JSON.parse(dump.toString('utf8'));
            if (parsed.format !== 'powermove-corpus-eval' || parsed.time !== sample.time) problems.push(`${at}: evaluation dump has format ${parsed.format} at ${parsed.time}`);
          } catch (error) { problems.push(`${at}: evaluation dump is not JSON: ${(error as Error).message}`); }
        }
      }
    }
  }
  const totals = manifest.totals ?? {};
  const expect = (name: string, actual: number) => { if (totals[name] !== actual) problems.push(`totals.${name} is ${totals[name]}, the project list gives ${actual}`); };
  expect('frames', frames); expect('floatFrames', floatFrames); expect('evalDumps', evalDumps);
  if (totals.blenderDumps !== undefined || blenderDumps) expect('blenderDumps', blenderDumps); expect('failedSamples', failedSamples); expect('bytes', bytes);
  expect('projects', (manifest.projects ?? []).length);

  const scenarios = manifest.scenarios;
  if (scenarios && !scenarios.error) {
    for (const [label, image] of Object.entries<any>(scenarios.images ?? {})) {
      const raw = check(image.raw, `scenario ${label} raw`);
      if (raw && sha256(raw) !== scenarios.hashes?.[label]) problems.push(`scenario ${label}: raw bytes do not match the recorded hash`);
      if (raw && raw.length !== image.width * image.height * 4) problems.push(`scenario ${label}: raw is ${raw.length} bytes for ${image.width}x${image.height}`);
      checkPng(image.png, `scenario ${label} png`, image.width, image.height);
    }
    if (Object.keys(scenarios.hashes ?? {}).length !== scenarios.count) problems.push(`scenarios.count is ${scenarios.count}`);
    listed.add('scenarios/rendering-performance/hashes.json').add('scenarios/rendering-performance/scenarios.json');
  }

  // The generated corpus: documents and media listed by generated/index.json.
  const generated = manifest.generated;
  if (generated?.dir) {
    const indexPath = path.join(dir, generated.dir, 'index.json');
    listed.add(`${generated.dir}/index.json`);
    if (!existsSync(indexPath)) problems.push(`${generated.dir}/index.json is missing`);
    else {
      const index = JSON.parse(readFileSync(indexPath, 'utf8'));
      for (const item of index.projects ?? []) {
        const file = `${generated.dir}/projects/${item.id}/document.json`;
        listed.add(file);
        const full = path.join(dir, file);
        if (!existsSync(full)) { problems.push(`${file} is missing`); continue; }
        checked++;
        if (sha256(readFileSync(full)) !== item.documentSha256) problems.push(`${file} does not match generated/index.json`);
      }
      for (const [name, media] of Object.entries<any>(generated.media ?? {})) {
        check({ file: `${generated.dir}/media/${name}`, bytes: media.bytes, sha256: media.sha256 }, `generated media ${name}`);
      }
    }
  }

  const unlisted: string[] = [];
  const walk = (relative: string) => {
    for (const entry of readdirSync(path.join(dir, relative), { withFileTypes: true })) {
      const child = relative ? `${relative}/${entry.name}` : entry.name;
      if (entry.isDirectory()) walk(child);
      else if (!listed.has(child)) unlisted.push(child);
    }
  };
  walk('');
  return { problems, checked, unlisted };
}

if (import.meta.main) {
  const dir = process.argv[2];
  if (!dir || !existsSync(dir) || !statSync(dir).isDirectory()) { console.error('usage: verify.ts <capture-dir>'); process.exit(2); }
  const result = verifyCapture(path.resolve(dir));
  for (const problem of result.problems) console.error(problem);
  for (const file of result.unlisted) console.error(`unlisted: ${file}`);
  console.log(`${result.checked} files checked, ${result.problems.length} problems, ${result.unlisted.length} unlisted files`);
  process.exit(result.problems.length || result.unlisted.length ? 1 : 0);
}
