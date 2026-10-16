/* Captures the rendering corpus: builds the capture pages, writes the cases,
   and runs the hidden Electron host (never shown, never focused, no Dock
   icon), which draws every case with today's Canvas2D code.

   ~/Library/Caches/powermove-engine/bin/pm-electron-lock bun engine/crates/pm-text/capture/render.ts [--set main|fit] [--out <dir>]
     -> <dir>/manifest.json, <dir>/<case>.rgba, <dir>/cases.json

   The default directory is a new `text-raster-<UTC stamp>` capture in the
   private corpus (`~/Library/Application Support/Powermove Engine Corpus`,
   or `$PM_ENGINE_CORPUS_ROOT`); the pixels are renders of the system's
   fonts and never enter the repository. */
import { mkdirSync, existsSync } from 'node:fs';
import { homedir } from 'node:os';
import { join, resolve } from 'node:path';

const crate = join(import.meta.dir, '..');
const repo = join(crate, '..', '..', '..');
const work = join(repo, 'engine', 'target', 'pm-text');
mkdirSync(work, { recursive: true });
const args = process.argv.slice(2);
const flag = (name: string) => { const i = args.indexOf(name); return i >= 0 ? args[i + 1] : undefined; };
const stamp = new Date().toISOString().replace(/[-:]/g, '').replace(/\..*/, '').replace('T', '-');
const corpusRoot = process.env.PM_ENGINE_CORPUS_ROOT ?? join(homedir(), 'Library', 'Application Support', 'Powermove Engine Corpus');
const set = flag('--set') ?? 'main';
const out = resolve(flag('--out') ?? join(corpusRoot, `${set === 'fit' ? 'text-fit' : 'text-raster'}-${stamp}`));
if (out.startsWith(resolve(repo) + '/')) throw new Error('refusing to write captured pixels inside the repository');
if (existsSync(join(out, 'manifest.json'))) throw new Error(`${out} already holds a capture`);
mkdirSync(out, { recursive: true });
const electron = join(repo, 'apps', 'desktop', 'node_modules', 'electron', 'dist', 'Electron.app', 'Contents', 'MacOS', 'Electron');
const env = { ...process.env };
delete env.ELECTRON_RUN_AS_NODE;
const run = async (cmd: string[]) => {
  const proc = Bun.spawn(cmd, { cwd: work, env, stdout: 'inherit', stderr: 'inherit' });
  const code = await proc.exited;
  if (code !== 0) throw new Error(`${cmd.join(' ')} exited ${code}`);
};
await run(['bun', join(import.meta.dir, 'build.ts'), work]);
await run(['bun', join(import.meta.dir, 'render-cases.ts'), join(out, 'cases.json'), ...(set === 'fit' ? ['fit'] : [])]);
await run([electron, join(import.meta.dir, 'electron-main.cjs'), `--page=${join(work, 'capture', 'render.html')}`,
  `--corpus=${join(out, 'cases.json')}`, `--out=${out}`, '--timeout=1800']);
console.log(`captured into ${out}`);
