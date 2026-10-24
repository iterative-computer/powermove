import { execFileSync } from 'node:child_process';
import { mkdtemp, rm } from 'node:fs/promises';
import { createRequire } from 'node:module';
import os from 'node:os';
import path from 'node:path';
import { build } from 'esbuild';
import { expect, it } from 'vitest';

it('releases an opened project after its caller and reader finish while keeping the file service alive', async () => {
  const directory = await mkdtemp(path.join(os.tmpdir(), 'powermove-project-retention-'));
  const appRoot = path.resolve(import.meta.dirname, '../..');
  try {
    const native = createRequire(path.join(appRoot, 'package.json')).resolve('@powermove/macos-haptics');
    const module = path.join(directory, 'project-files.cjs');
    await build({ entryPoints: [path.join(import.meta.dirname, 'project-files.ts')], outfile: module,
      bundle: true, platform: 'node', format: 'cjs', plugins: [{ name: 'native-addon-location', setup(builder) {
        builder.onResolve({ filter: /^@powermove\/macos-haptics$/ }, () => ({ path: native, external: true }));
      } }] });
    const result = JSON.parse(execFileSync(process.execPath, ['--expose-gc', '-e', `
      const fs = require('node:fs/promises'), path = require('node:path');
      const { ProjectFiles } = require(${JSON.stringify(module)});
      const directory = ${JSON.stringify(directory)};
      const turn = () => new Promise(resolve => setImmediate(resolve));
      async function inspect(files, file) {
        let opened = await files.open(file);
        const weak = new WeakRef(opened.document.history);
        await files.close(opened.token);
        await turn(); global.gc();
        // The caller still owns its returned document after closing the reader.
        if (!weak.deref() || opened.document.history.entries.length !== 20_000) throw new Error('Caller lost its project');
        opened = null;
        return weak;
      }
      (async () => {
        const file = path.join(directory, 'synthetic.pmv');
        const entry = JSON.stringify({ label: 'Move synthetic layer', before: { x: 0 }, after: { x: 100 },
          note: 'Synthetic history fixture' });
        await fs.writeFile(file, '{"proj":{"w":100,"h":100,"layers":[]},"history":{"entries":['
          + Array(20_000).fill(entry).join(',') + ']}}');
        const files = new ProjectFiles(path.join(directory, 'state/files.json'), path.join(directory, 'backups'));
        const weak = await inspect(files, file);
        for (let pass = 0; pass < 5; pass++) { await turn(); global.gc(); }
        const retained = !!weak.deref();
        // Do not use another queued operation to release the old result.
        // An idle, live service must no longer own the imported history.
        await files.destination('unknown');
        console.log(JSON.stringify({ retained }));
      })().catch(error => { console.error(error); process.exit(1); });
    `], { cwd: directory, encoding: 'utf8', timeout: 10_000 }));
    expect(result).toEqual({ retained: false });
  } finally { await rm(directory, { recursive: true, force: true }); }
}, 15_000);
