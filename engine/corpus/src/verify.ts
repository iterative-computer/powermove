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

  // BLEND-1 adds retained references beside active frames. Keep every record
  // inside the capture even if a manifest is edited to name an absolute path,
  // `..`, or a symlink into another directory.
  const checkBlend = (record: unknown, what: string): Buffer | null => {
    if (!isRecord(record) || !Number.isSafeInteger(record.bytes) || record.bytes < 0) {
      problems.push(`${what}: malformed file record`);
      return null;
    }
    const relative = record.file;
    if (!relative || path.isAbsolute(relative) || relative.split(/[\\/]/).includes('..')) {
      problems.push(`${what}: record is outside the capture: ${relative}`);
      return null;
    }
    const file = path.resolve(dir, relative);
    if (existsSync(file)) {
      const root = realpathSync(dir), resolved = realpathSync(file);
      const fromRoot = path.relative(root, resolved);
      if (!fromRoot || fromRoot.startsWith('..') || path.isAbsolute(fromRoot)) {
        problems.push(`${what}: record is outside the capture: ${relative}`);
        return null;
      }
    }
    return check(record, what);
  };
  const blendPng = (record: unknown, what: string, width: number, height: number): Uint8Array | null => {
    const bytes = checkBlend(record, what);
    if (!bytes) return null;
    try {
      const png = decodePng(new Uint8Array(bytes));
      if (png.width !== width || png.height !== height || png.channels !== 4) {
        problems.push(`${what}: ${png.width}x${png.height}x${png.channels}, expected ${width}x${height}x4`);
        return null;
      }
      return png.data;
    } catch (error) { problems.push(`${what}: PNG does not decode: ${(error as Error).message}`); return null; }
  };
  const blendFloat = (value: any, what: string, width: number, height: number, time: number): Buffer | null => {
    const data = checkBlend(value?.data, `${what} data`);
    const header = checkBlend(value?.header, `${what} header`);
    if (!data || !header) return null;
    if (data.length !== width * height * 16) problems.push(`${what}: float data is ${data.length} bytes, expected ${width * height * 16}`);
    try {
      const parsed = JSON.parse(header.toString('utf8'));
      if (parsed.format !== 'powermove-corpus-rgba32f' || parsed.version !== 1
          || parsed.width !== width || parsed.height !== height || parsed.time !== time
          || parsed.channels !== 4 || parsed.sampleType !== 'f32' || parsed.endianness !== 'little'
          || parsed.rowOrder !== 'top-down' || parsed.alpha !== 'premultiplied'
          || parsed.sourcePrecision !== 'binary16'
          || parsed.dataFile !== path.basename(value.data.file) || parsed.dataBytes !== data.length
          || parsed.dataSha256 !== value.data.sha256) problems.push(`${what}: float header differs from data or sample`);
    } catch (error) { problems.push(`${what}: float header is not JSON: ${(error as Error).message}`); }
    if (data.length === width * height * 16) {
      const view = new DataView(data.buffer, data.byteOffset, data.byteLength);
      let nonFinite = 0;
      for (let offset = 0; offset < data.length; offset += 4) if (!Number.isFinite(view.getFloat32(offset, true))) nonFinite++;
      if (nonFinite) problems.push(`${what}: ${nonFinite} non-finite float values`);
    }
    return data;
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
      const recaptures = sample.blendMainRecapture;
      if (sample.error) failedSamples++;
      if (sample.png) { frames++; bytes += sample.png.bytes; }
      if (recaptures !== undefined && sample.png) blendPng(sample.png, `${at} png`, sample.width, sample.height);
      else checkPng(sample.png, `${at} png`, sample.width, sample.height);
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
      const local = localSamples.find((s: any) => s.time === sample.time);
      if (!isDeepStrictEqual(recaptures, local?.blendMainRecapture)) problems.push(`${at}: BLEND-1 metadata differs between project and manifest`);
      if (recaptures !== undefined) {
        if (!recaptures || typeof recaptures !== 'object' || Array.isArray(recaptures)
            || !Object.keys(recaptures).length) problems.push(`${at}: invalid BLEND-1 annotation`);
        else for (const [kind, annotation] of Object.entries<any>(recaptures)) {
          const label = `${at} BLEND-1 ${kind}`;
          if (kind !== 'opaque' && kind !== 'float') { problems.push(`${label}: invalid comparison kind`); continue; }
          if (!annotation || annotation.id !== 'BLEND-1' || annotation.mainCommit !== '2b1fa379a'
              || annotation.reason !== 'recaptured after main 2b1fa379a blend change'
              || !/^[0-9a-f]{64}$/.test(annotation.oldCaptureSha256)
              || !/^[0-9a-f]{64}$/.test(annotation.currentCaptureSha256)
              || !/^[0-9a-f]{40}$/.test(annotation.mergedMainHead)
              || !/^[0-9a-f]{64}$/.test(annotation.appBuildSha256)
              || annotation.oldCaptureSha256 === annotation.currentCaptureSha256) {
            problems.push(`${label}: invalid decision metadata`);
            continue;
          }
          const active = kind === 'opaque' ? sample.png : sample.float;
          const localActive = kind === 'opaque' ? local?.png : local?.float;
          if (!isDeepStrictEqual(annotation.corrected, active) || !isDeepStrictEqual(active, localActive)) problems.push(`${label}: corrected record differs from active sample`);
          const oldPixels = kind === 'opaque'
            ? blendPng(annotation.original, `${label} original`, sample.width, sample.height)
            : blendFloat(annotation.original, `${label} original`, sample.width, sample.height, sample.time);
          const currentPixels = kind === 'opaque'
            ? blendPng(annotation.corrected, `${label} corrected`, sample.width, sample.height)
            : blendFloat(annotation.corrected, `${label} corrected`, sample.width, sample.height, sample.time);
          const historical = checkBlend(annotation.historical, `${label} historical`);
          if (historical && historical.length !== sample.width * sample.height * (kind === 'opaque' ? 4 : 16)) problems.push(`${label}: historical raw data has wrong dimensions`);
          if (oldPixels && sha256(oldPixels) !== annotation.oldCaptureSha256) problems.push(`${label}: original pixels differ from oldCaptureSha256`);
          if (historical && sha256(historical) !== annotation.oldCaptureSha256) problems.push(`${label}: historical pixels differ from oldCaptureSha256`);
          if (currentPixels && sha256(currentPixels) !== annotation.currentCaptureSha256) problems.push(`${label}: corrected pixels differ from currentCaptureSha256`);
          const proof = checkBlend(annotation.evidence, `${label} evidence`);
          if (proof) {
            try {
              const evidence = JSON.parse(proof.toString('utf8'));
              if (evidence.id !== 'BLEND-1' || evidence.mainCommit !== annotation.mainCommit
                  || evidence.reason !== annotation.reason || evidence.kind !== kind
                  || evidence.projectId !== project.id || evidence.time !== sample.time
                  || !isDeepStrictEqual(evidence.original, annotation.original)
                  || !isDeepStrictEqual(evidence.current, annotation.corrected)
                  || !isDeepStrictEqual(evidence.historical, annotation.historical)
                  || evidence.oldCaptureSha256 !== annotation.oldCaptureSha256
                  || evidence.originalPixelSha256 !== annotation.oldCaptureSha256
                  || evidence.currentCaptureSha256 !== annotation.currentCaptureSha256
                  || evidence.loadedDocumentSha256 !== project.loaded?.sha256
                  || evidence.mergedMainHead !== annotation.mergedMainHead
                  || evidence.appBuild?.sha256 !== annotation.appBuildSha256
                  || evidence.oldExact !== true || evidence.currentRepeatExact !== true
                  || !isDeepStrictEqual(evidence.equation, {
                    old: ['vec3 res = mix(dc, bc, s.a);', 'o = vec4(res * a, a);'],
                    current: ['vec3 res = (1. - s.a) * d.rgb + (1. - d.a) * s.rgb + s.a * d.a * bc;', 'o = vec4(res, a);'],
                  })
                  || !isDeepStrictEqual(evidence.setting, { useNewEngine: false, colorDepth: 16 })) {
                problems.push(`${label}: evidence differs from annotation or sample`);
              }
            } catch (error) { problems.push(`${label}: evidence is not JSON: ${(error as Error).message}`); }
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
        if (float === sample.float && recaptures !== undefined) {
          blendFloat(float, `${at} float`, sample.width, sample.height, sample.time);
          continue;
        }
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
