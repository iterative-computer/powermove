// Build external G-T5 replay input from passing G-T1/G-T2 reference reports.
// Usage: node text-reference-manifest.mjs --raster-report <json> --raster-pixels <dir>
//   --frame-report <json> --frame-pixels <dir> --out <json> [--limit 8]
// Reports and output remain outside the public repository.
import { readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const argv = process.argv.slice(2);
const options = Object.fromEntries(Array.from({ length: Math.floor(argv.length / 2) }, (_, i) =>
  [argv[i * 2]?.replace(/^--/, ''), argv[i * 2 + 1]]));
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../../..');
const outsideGit = value => {
  const absolute = path.resolve(value);
  if (absolute === root || absolute.startsWith(`${root}${path.sep}`)) {
    throw new Error('Text evidence must remain outside the public repository');
  }
  return absolute;
};
const required = name => {
  if (!options[name]) throw new Error(`Missing --${name}`);
  return outsideGit(options[name]);
};
const output = required('out');
const limit = options.limit === undefined ? Infinity : Number(options.limit);
if (!(limit > 0 && (Number.isSafeInteger(limit) || limit === Infinity))) {
  throw new Error('--limit must be a positive integer');
}
const json = async file => JSON.parse(await readFile(file, 'utf8'));
const rasters = [];
const frames = [];

if (options['raster-report'] || options['raster-pixels']) {
  const report = required('raster-report');
  const pixels = required('raster-pixels');
  const groups = new Map();
  const caseFiles = new Map();
  for (const row of await json(report)) {
    if (!row.pass || row.referenceModel !== 'outline') continue;
    if (typeof row.captureDir !== 'string' || !Number.isSafeInteger(row.captureCaseIndex)) {
      throw new Error('G-T1 report needs captureDir and captureCaseIndex; rerun the current gate');
    }
    const capture = outsideGit(row.captureDir);
    let group = groups.get(capture);
    if (!group) {
      group = { name: `numeric-raster-${groups.size}`, dir: capture, ids: [],
        reference: { report, pixels, indices: [] } };
      groups.set(capture, group);
    }
    if (group.ids.length >= limit) continue;
    let cases = caseFiles.get(capture);
    if (!cases) {
      cases = await json(path.join(capture, 'cases.json'));
      caseFiles.set(capture, cases);
    }
    const id = cases.cases[row.captureCaseIndex]?.id;
    if (typeof id !== 'string') throw new Error(`Missing G-T1 case ${row.case}`);
    group.ids.push(id);
    group.reference.indices.push(row.case);
  }
  rasters.push(...groups.values());
}

if (options['frame-report'] || options['frame-pixels']) {
  const report = required('frame-report');
  const pixels = required('frame-pixels');
  const groups = new Map();
  for (const row of await json(report)) {
    if (!row.pass || row.referenceModel !== 'outline') continue;
    if (typeof row.captureDir !== 'string' || typeof row.projectDir !== 'string'
      || !Number.isSafeInteger(row.sampleIndex)) {
      throw new Error('G-T2 report needs captureDir, projectDir and sampleIndex; rerun the current gate');
    }
    const capture = outsideGit(row.captureDir);
    const key = `${capture}\0${row.projectDir}`;
    let group = groups.get(key);
    if (!group) {
      group = { name: `numeric-frame-${groups.size}`, dir: capture, project: row.projectDir,
        times: [], reference: { report, pixels, indices: [] } };
      groups.set(key, group);
    }
    if (group.times.length >= limit) continue;
    const project = await json(path.join(capture, row.projectDir, 'project.json'));
    const time = project.samples[row.sampleIndex]?.time;
    if (typeof time !== 'number' || time !== row.time) {
      throw new Error(`G-T2 sample mismatch at numeric frame ${row.case}`);
    }
    group.times.push(time);
    group.reference.indices.push(row.case);
  }
  frames.push(...groups.values());
}

await writeFile(output, `${JSON.stringify({ rasters, frames }, null, 2)}\n`);
console.log(`G-T5 manifest: ${rasters.reduce((n, g) => n + g.ids.length, 0)} raster cases, `
  + `${frames.reduce((n, g) => n + g.times.length, 0)} frame samples`);
