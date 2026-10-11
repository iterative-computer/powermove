/* Builds the capture pages and runs the hidden Electron host.
   bun capture/run.ts measure   -> out/corpus.json, out/canvas2d.json, out/platform-fonts.json
   bun capture/run.ts render    -> out/render-canvas2d/*.png (scenes from out/scenes.json) */
import { join } from 'node:path';

const root = join(import.meta.dir, '..');
const repo = join(root, '..', '..', '..');
const electron = join(repo, 'apps', 'desktop', 'node_modules', 'electron', 'dist', 'Electron.app', 'Contents', 'MacOS', 'Electron');
const mode = process.argv[2] ?? 'measure';
// The parent shell may run inside an Electron app with ELECTRON_RUN_AS_NODE set,
// which would start the capture host as plain Node.
const env = { ...process.env };
delete env.ELECTRON_RUN_AS_NODE;
const run = async (cmd: string[]) => {
  const proc = Bun.spawn(cmd, { cwd: root, env, stdout: 'inherit', stderr: 'inherit' });
  const code = await proc.exited;
  if (code !== 0) throw new Error(`${cmd.join(' ')} exited ${code}`);
};
await run(['bun', join(import.meta.dir, 'build.ts')]);
if (mode === 'measure') {
  await run(['bun', join(import.meta.dir, 'corpus.ts'), join(root, 'out', 'corpus.json')]);
  await run([electron, join(import.meta.dir, 'electron-main.cjs'), '--mode=measure', '--corpus=out/corpus.json', '--out=out/canvas2d.json', '--probe-fonts=out/platform-fonts.json']);
} else if (mode === 'render') {
  await run([electron, join(import.meta.dir, 'electron-main.cjs'), '--mode=render', '--scenes=out/scenes.json', '--out=out/render-canvas2d', ...process.argv.slice(3)]);
} else throw new Error(`unknown mode ${mode}`);
