// Golden PMV4 files written by the OLD TypeScript writer (`ProjectSave` from
// `main:apps/desktop/src/main/project-save.ts`), for tests/ts_writer_parity.rs.
//
// Regenerate (from the repository root; needs bun):
//   mkdir -p /tmp/pm-main-src && git archive main \
//     apps/desktop/src/main/project-save.ts apps/desktop/src/main/durable-file.ts \
//     apps/desktop/src/main/file-upload.ts apps/desktop/src/main/read-retry.ts \
//     apps/desktop/src/shared/project-incremental.ts apps/desktop/src/shared/sha256.ts \
//     | tar -x -C /tmp/pm-main-src
//   bun apps/native/crates/core/tests/fixtures/ts-writer/generate.ts /tmp/pm-main-src
//
// Every object below has its keys in canonical (code unit) order, so
// `JSON.stringify` writes exactly the canonical serialization the Rust writer
// emits; the envelope is built the way `legacy/app.ts` builds it
// (`{"v":..,"proj":..,"history":..,"ws":..}`).
import { copyFileSync, mkdtempSync, readFileSync, renameSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';

const root = process.argv[2];
if (!root) throw new Error('usage: bun generate.ts <extracted main tree>');
const { ProjectSave } = await import(path.join(root, 'apps/desktop/src/main/project-save.ts'));
const { fileVersion } = await import(path.join(root, 'apps/desktop/src/main/durable-file.ts'));

const here = import.meta.dir;
const sorted = (value: any): any => Array.isArray(value) ? value.map(sorted)
  : value && typeof value === 'object' ? Object.fromEntries(Object.keys(value).sort().map(k => [k, sorted(value[k])])) : value;

const channel = (v: number) => ({ expr: null, kf: [], v });
const project = (opacity: number) => sorted({
  id: 'P', name: 'Golden', compId: 'C1', compName: 'Main', w: 64, h: 36, fps: 30, dur: 2, bg: '#000000',
  layers: [
    { id: 'a', type: 'image', name: 'Still', from: 0, dur: 2, on: true, lock: false, d: { asset: 'img' }, p: { opacity: channel(opacity) } },
    { id: 'b', type: 'video', name: 'Clip', from: 0.5, dur: 1.5, on: true, lock: false, d: { asset: 'clip' }, p: { opacity: channel(100) } },
  ],
  assets: { img: { id: 'img', kind: 'image', name: 'still.png' }, clip: { id: 'clip', kind: 'video', name: 'clip.webm' } },
  comps: { C2: { id: 'C2', name: 'Other', w: 32, h: 32, fps: 30, dur: 1, layers: [], futureCompField: { x: 1 } } },
  markers: [], work: [0, 2],
  futureField: { keep: ['me', 1.5, null, true] },
  text: 'lone \ud800 surrogate and 😀',
});
const path0 = ['comps', 'C1', 'layers', 0, 'p', 'opacity', 'v'];
const entry = (label: string, from: number, to: number) => sorted({
  label, forward: [{ exists: true, path: path0, value: to }], backward: [{ exists: true, path: path0, value: from }],
});
const history1 = sorted({ version: 1, paths: 'composition', index: 0, entries: [entry('Change Opacity', 100, 80)] });
const history2 = sorted({ version: 1, paths: 'composition', index: 1, entries: [entry('Change Opacity', 100, 80), entry('Change Opacity', 80, 50)] });
const ws = sorted({ panels: ['viewer', 'timeline'], zoom: 1.25 });
const envelope = (proj: any, history: any) => `{"v":${JSON.stringify('1.0.0')},"proj":${JSON.stringify(proj)},"history":${JSON.stringify(history)},"ws":${JSON.stringify(ws)}}`;

const mediaA = new Uint8Array([137, 80, 78, 71, 13, 10, 26, 10, 1, 2, 3]);
const mediaB = new Uint8Array(3000).map((_, i) => (i * 7 + 3) & 255);
const media = [
  { id: 'img', revision: 'r1', type: 'image/png', length: mediaA.length, data: mediaA },
  { id: 'clip', revision: 'r2', type: 'video/webm', length: mediaB.length, data: mediaB },
];

async function save(destination: string, documentText: string, reusePrevious: boolean) {
  const previous = await fileVersion(destination);
  const doc = new TextEncoder().encode(documentText);
  const commit = async (temporary: string) => { renameSync(temporary, destination); return destination; };
  const save = await ProjectSave.create(destination, previous, media.map(({ data, ...m }) => m), doc.length, commit, () => {}, reusePrevious);
  for (let o = 0; o < doc.length; o += 1024 * 1024) await save.write(null, doc.subarray(o, o + 1024 * 1024));
  for (const id of save.required) {
    const item = media.find(m => m.id === id)!;
    await save.write(id, item.data);
  }
  await save.finish();
}

const work = mkdtempSync(path.join(tmpdir(), 'pm-golden-'));
try {
  // 1. A first save: no previous file, so the fresh layout (document, then media).
  const fresh = path.join(work, 'fresh.pmv');
  await save(fresh, envelope(project(80), history1), true);
  copyFileSync(fresh, path.join(here, 'fresh.pmv'));
  // 2. Saving the same file again after an edit: both media records are
  //    reused, little dead data, so ProjectSave clones the file and appends
  //    the new document, index and footer.
  await save(fresh, envelope(project(50), history2), true);
  copyFileSync(fresh, path.join(here, 'append.pmv'));
  // 3. Save As of the edited project to a new name: no reuse, fresh layout.
  const saveAs = path.join(work, 'save-as.pmv');
  await save(saveAs, envelope(project(50), history2), false);
  copyFileSync(saveAs, path.join(here, 'save-as.pmv'));
  writeFileSync(path.join(here, 'media-img.bin'), mediaA);
  writeFileSync(path.join(here, 'media-clip.bin'), mediaB);
} finally { rmSync(work, { recursive: true, force: true }); }
console.log('wrote fresh.pmv, append.pmv, save-as.pmv');
