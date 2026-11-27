/** Read public corpus source; write ordinal-only disposable copies outside Git. */
import { allProjects } from '../../engine/corpus/src/projects';
import { mediaRefs } from '../../engine/corpus/src/media';
import { validateDocument } from '../../engine/corpus/src/validate';
import { mkdirSync, writeFileSync, realpathSync } from 'node:fs';
import { createHash } from 'node:crypto';
import path from 'node:path';
const out = path.resolve(process.argv[2] ?? '');
mkdirSync(out, { recursive: true });
if (!realpathSync(out).startsWith('/private/tmp/')) throw new Error('Output must be a temporary directory');
const projects = allProjects();
// A representative, extension-free visual composition; no media hydration changes either workload.
const ordinal = projects.findIndex(p => !p.extensions?.length && !mediaRefs(p.doc).size &&
  p.doc.layers?.length >= 3 && p.doc.layers.every((l: any) => l.type === 'shape') &&
  !p.doc.layers.some((l: any) => l.fx?.length));
if (ordinal < 0) throw new Error('No extension-free public shape fixture');
const doc = projects[ordinal]!.doc;
if (validateDocument(doc).length) throw new Error('Invalid corpus document');
const bytes = JSON.stringify(doc);
writeFileSync(path.join(out, 'document.json'), bytes);
writeFileSync(path.join(out, 'fixture.json'), JSON.stringify({ ordinal, sha256: createHash('sha256').update(bytes).digest('hex'),
  count: projects.length, source: 'engine/corpus', variant: 'unaltered', width: doc.w, height: doc.h, fps: doc.fps, duration: doc.dur }));

// Node-compatible version of the tag's own e2e helper, as in trace-recorder/apply.ts.
const ref = path.resolve(process.argv[3] ?? '/tmp/pm-ref-v1.2.0');
const helper = path.join(ref, 'apps/desktop/e2e/helpers/app.ts');
const original = await Bun.file(helper).text();
for (const anchor of ['_electron.launch({', '// Native dialogs are substituted', 'return { app, page };']) {
  if (original.indexOf(anchor) < 0 || original.indexOf(anchor) !== original.lastIndexOf(anchor)) throw new Error(`Unsupported helper anchor: ${anchor}`);
}
const source = original.replace('_electron.launch({', '_electron.launch({ timeout: 30_000,')
  .replace('// Native dialogs are substituted', 'app.context().setDefaultTimeout(20_000);\ntry {\n// Native dialogs are substituted')
  .replace('return { app, page };', 'return { app, page };\n} catch(error) { await app.close().catch(()=>{}); throw error; }');
const build = await Bun.build({entrypoints:[helper], target:'node', packages:'external', define:{__dirname:JSON.stringify(path.dirname(helper))},
  plugins:[{name:'perf-harness-cli',setup(b){b.onLoad({filter:/e2e\/helpers\/app\.ts$/},()=>({contents:source,loader:'ts'}));}}]});
if (!build.success) throw new AggregateError(build.logs, 'Helper transpilation failed');
writeFileSync(path.join(path.dirname(helper), 'app.perf.mjs'), await build.outputs[0]!.text());
console.log('Prepared public corpus copy and isolated reference helper');
