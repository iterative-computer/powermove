/* Bundles the capture page into <root>/capture. Usage: bun capture/build.ts <root> */
import { mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

const root = process.argv[2];
if (!root) throw new Error('usage: bun build.ts <root>');
const outDir = join(root, 'capture');
mkdirSync(outDir, { recursive: true });
for (const [name, entry] of [['measure', 'page.ts']] as const) {
  const result = await Bun.build({ entrypoints: [join(import.meta.dir, entry)], target: 'browser', format: 'iife', minify: false });
  if (!result.success) { console.error(result.logs); process.exit(1); }
  writeFileSync(join(outDir, `${name}.js`), await result.outputs[0]!.text());
  writeFileSync(join(outDir, `${name}.html`), `<!doctype html><meta charset="utf-8"><body style="margin:0;background:#000"><script src="./${name}.js"></script>`);
}
console.log(`built capture pages in ${outDir}`);
