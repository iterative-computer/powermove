/* Builds the capture page and runs the hidden Electron host (never shown,
   never focused, no Dock icon).
   bun engine/crates/pm-text/capture/run.ts
     -> engine/target/pm-text/corpus.json, canvas2d.json, platform-fonts.json */
import { mkdirSync } from 'node:fs';
import { join } from 'node:path';

const crate = join(import.meta.dir, '..');
const repo = join(crate, '..', '..', '..');
const root = join(repo, 'engine', 'target', 'pm-text');
mkdirSync(root, { recursive: true });
// The Electron app ships from main; its node_modules live in a v1 checkout at PM_V1_REF (default /tmp/pm-ref-v1.2.0).
const v1Root = process.env.PM_V1_REF ?? '/tmp/pm-ref-v1.2.0';
const electron = join(v1Root, 'apps', 'desktop', 'node_modules', 'electron', 'dist', 'Electron.app', 'Contents', 'MacOS', 'Electron');
// The parent shell may run inside an Electron app with ELECTRON_RUN_AS_NODE set,
// which would start the capture host as plain Node.
const env = { ...process.env };
delete env.ELECTRON_RUN_AS_NODE;
const run = async (cmd: string[]) => {
  const proc = Bun.spawn(cmd, { cwd: root, env, stdout: 'inherit', stderr: 'inherit' });
  const code = await proc.exited;
  if (code !== 0) throw new Error(`${cmd.join(' ')} exited ${code}`);
};
await run(['bun', join(import.meta.dir, 'build.ts'), root]);
await run(['bun', join(import.meta.dir, 'corpus.ts'), join(root, 'corpus.json')]);
await run([electron, join(import.meta.dir, 'electron-main.cjs'), `--page=${join(root, 'capture', 'measure.html')}`,
  `--corpus=${join(root, 'corpus.json')}`, `--out=${join(root, 'canvas2d.json')}`, `--probe-fonts=${join(root, 'platform-fonts.json')}`]);
