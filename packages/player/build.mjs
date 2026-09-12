// Bundles the Powermove web player from the desktop app's sources.
// The esbuild options here are the single source of truth: the desktop app's
// `virtual:powermove-player` module (embedded into web exports) and this
// package's dist/ output both go through bundlePlayerEntry.
import { build } from 'esbuild';
import { copyFile, mkdir, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const packageDir = path.dirname(fileURLToPath(import.meta.url));
const playerSourceDir = path.resolve(packageDir, '../../apps/desktop/src/renderer/src/player');

export const entries = {
  player: path.join(playerSourceDir, 'player.ts'),
  'svg-player': path.join(playerSourceDir, 'svg-player.ts'),
};

/**
 * Bundle one browser-only entry to ESM text.
 * @param {string} entryPath absolute path to the entry module
 * @returns {Promise<{ text: string; inputs: string[] }>} bundle text and the absolute paths of every input file
 */
export async function bundlePlayerEntry(entryPath) {
  const result = await build({
    entryPoints: [entryPath],
    bundle: true, write: false, format: 'esm', platform: 'browser', target: 'es2022',
    minify: true, legalComments: 'inline', define: { 'import.meta.hot': 'false' },
    metafile: true,
    // esbuild reports metafile inputs relative to its working directory; pin it so callers get stable paths.
    absWorkingDir: packageDir,
  });
  const inputs = Object.keys(result.metafile.inputs).map(file => path.resolve(packageDir, file));
  return { text: result.outputFiles[0].text, inputs };
}

async function main() {
  const dist = path.join(packageDir, 'dist');
  await mkdir(dist, { recursive: true });
  for (const [name, entry] of Object.entries(entries)) {
    const { text } = await bundlePlayerEntry(entry);
    await writeFile(path.join(dist, `${name}.js`), text);
    // Declarations are hand-written (the sources are loosely typed `any` engines); ship them beside the bundles.
    await copyFile(path.join(packageDir, 'types', `${name}.d.ts`), path.join(dist, `${name}.d.ts`));
    console.log(`dist/${name}.js ${(text.length / 1024).toFixed(1)} kB, dist/${name}.d.ts`);
  }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch(error => { console.error(error); process.exit(1); });
}
