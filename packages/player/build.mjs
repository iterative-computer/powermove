// Bundles the Powermove web player from the Electron app's player sources.
// The Electron app ships from main; this branch no longer carries it, so the
// sources come from the pinned commit below (read with `git archive`, which
// needs the repository's history, not a shallow clone).
import { build } from 'esbuild';
import { execFileSync } from 'node:child_process';
import { copyFile, mkdir, writeFile } from 'node:fs/promises';
import { existsSync, mkdirSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const SOURCE_COMMIT = '0c818d97c398a34a9e6e11bb1da71cf94b4ee0e2';
const packageDir = path.dirname(fileURLToPath(import.meta.url));
const sources = path.join(packageDir, '.v1-src', SOURCE_COMMIT);

function extractSources() {
  if (existsSync(path.join(sources, 'apps/desktop/src'))) return;
  mkdirSync(sources, { recursive: true });
  const archive = execFileSync('git', ['archive', SOURCE_COMMIT, 'apps/desktop/src'], { cwd: path.resolve(packageDir, '../..'), maxBuffer: 256 * 1024 * 1024 });
  execFileSync('tar', ['-x', '-C', sources], { input: archive });
}

async function main() {
  extractSources();
  const playerSourceDir = path.join(sources, 'apps/desktop/src/renderer/src/player');
  const dist = path.join(packageDir, 'dist');
  await mkdir(dist, { recursive: true });
  for (const name of ['player', 'svg-player']) {
    const result = await build({
      entryPoints: [path.join(playerSourceDir, `${name}.ts`)],
      bundle: true, write: false, format: 'esm', platform: 'browser', target: 'es2022',
      minify: true, legalComments: 'inline', define: { 'import.meta.hot': 'false' },
      nodePaths: [path.join(packageDir, 'node_modules')],
    });
    const text = result.outputFiles[0].text;
    await writeFile(path.join(dist, `${name}.js`), text);
    // Declarations are hand-written (the sources are loosely typed `any` engines); ship them beside the bundles.
    await copyFile(path.join(packageDir, 'types', `${name}.d.ts`), path.join(dist, `${name}.d.ts`));
    console.log(`dist/${name}.js ${(text.length / 1024).toFixed(1)} kB, dist/${name}.d.ts`);
  }
}

main().catch(error => { console.error(error); process.exit(1); });
