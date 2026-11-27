/* Bundles the capture page into <root>/capture. Usage: bun capture/build.ts <root> */
import { mkdirSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';

// The Electron app ships from main; its v1 source (the font renderer and text
// layout) lives in a checkout at PM_V1_REF (default /tmp/pm-ref-v1.2.0).
const v1Root = resolve(process.env.PM_V1_REF ?? '/tmp/pm-ref-v1.2.0');
const v1Resolve = { name: 'v1-desktop', setup(b: any) { b.onResolve({ filter: /apps\/desktop\// }, (a: { path: string }) => ({ path: resolve(v1Root, a.path.slice(a.path.indexOf('apps/desktop/'))) })); } };

const root = process.argv[2];
if (!root) throw new Error('usage: bun build.ts <root>');
const outDir = join(root, 'capture');
mkdirSync(outDir, { recursive: true });
for (const [name, entry] of [['measure', 'page.ts']] as const) {
  const result = await Bun.build({ entrypoints: [join(import.meta.dir, entry)], target: 'browser', format: 'iife', minify: false, plugins: [v1Resolve] });
  if (!result.success) { console.error(result.logs); process.exit(1); }
  writeFileSync(join(outDir, `${name}.js`), await result.outputs[0]!.text());
  writeFileSync(join(outDir, `${name}.html`), `<!doctype html><meta charset="utf-8"><body style="margin:0;background:#000"><script src="./${name}.js"></script>`);
}
console.log(`built capture pages in ${outDir}`);
