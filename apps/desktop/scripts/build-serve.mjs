/*
 * Builds the `powermove serve` host: the main-process modules bundled for plain
 * Node with `electron` aliased to the stub, plus the renderer and the resource
 * folders the packaged app ships. Output lands in packages/cli/dist by default.
 *
 *   node scripts/build-serve.mjs [--out <dir>] [--skip-renderer]
 */
import { build } from 'esbuild';
import { cp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const desktop = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const args = process.argv.slice(2);
const outIndex = args.indexOf('--out');
const outDir = path.resolve(outIndex >= 0 ? args[outIndex + 1] : path.join(desktop, '../../packages/cli/dist'));
const skipRenderer = args.includes('--skip-renderer');
const pkg = JSON.parse(await readFile(path.join(desktop, 'package.json'), 'utf8'));

if (!skipRenderer) {
  const result = spawnSync('bun', ['x', 'electron-vite', 'build'], { cwd: desktop, stdio: 'inherit' });
  if (result.status !== 0) process.exit(result.status ?? 1);
}

await rm(outDir, { recursive: true, force: true });
await mkdir(outDir, { recursive: true });

await build({
  entryPoints: [path.join(desktop, 'src/server/cli.ts')],
  bundle: true,
  platform: 'node',
  target: 'node22',
  format: 'esm',
  outfile: path.join(outDir, 'server.mjs'),
  // Bundle the shared TypeScript packages and their JS dependencies. The
  // standalone npm package must not depend on unpublished workspace modules.
  // Keep native/process-backed dependencies in node_modules.
  external: ['@anthropic-ai/claude-agent-sdk', 'esbuild', 'ws', 'selfsigned', '@powermove/macos-haptics'],
  alias: { electron: path.join(desktop, 'src/server/electron-stub.ts') },
  define: { 'process.env.NODE_ENV': '"production"' },
  sourcemap: true,
  logLevel: 'info',
  banner: { js: '// Powermove serve host. Built from apps/desktop/src/server by scripts/build-serve.mjs.' }
});

await cp(path.join(desktop, 'out/renderer'), path.join(outDir, 'renderer'), { recursive: true });

// The document engine: the renderer's core built for Node by Vite (SSR).
{
  const result = spawnSync('bun', ['x', 'vite', 'build', '--config', 'engine.vite.config.ts'], { cwd: desktop, stdio: 'inherit', env: { ...process.env, POWERMOVE_ENGINE_OUT: path.join(outDir, 'engine') } });
  if (result.status !== 0) process.exit(result.status ?? 1);
}

// Mirrors electron-builder.yml extraResources for what the host needs.
const resources = path.join(outDir, 'resources');
const copies = [
  ['src/extensions', 'builtin-extensions'],
  ['docs/EXTENSIONS.md', 'api-pack/EXTENSIONS.md'],
  ['src/shared/blender.ts', 'api-pack/blender-schema.ts'],
  ['src/renderer/src/core/scene3d/materials.ts', 'api-pack/scene3d-materials.ts'],
  ['src/renderer/src/core/scene3d/modeling.ts', 'api-pack/scene3d-modeling.ts'],
  ['docs/SCENE3D.md', 'api-pack/SCENE3D.md'],
  ['src/renderer/src/core/scene3d/schema.ts', 'api-pack/scene3d-schema.ts'],
  ['src/renderer/src/core/scene3d/layers.ts', 'api-pack/scene3d-layers.ts'],
  ['src/renderer/src/core/scene3d/operations.ts', 'api-pack/scene3d-operations.ts'],
  ['docs/background-testing.md', 'api-pack/BACKGROUND_TESTING.md'],
  ['src/renderer/src/kernel/api.ts', 'api-pack/api.ts'],
  ['../../packages/registry/src/manifest.ts', 'api-pack/extensions.ts'],
  ['src/renderer/src/core/types/project.ts', 'api-pack/project.ts'],
  ['src/renderer/src/core/types/commands.ts', 'api-pack/commands.ts'],
  ['docs/samples/media-browser', 'api-pack/samples/media-browser'],
  ['docs/samples/gradient-tint', 'api-pack/samples/gradient-tint'],
  ['src/main/agent-tools/mcp-server.mjs', 'agent-tools/mcp-server.mjs']
];
for (const [from, to] of copies) {
  const target = path.join(resources, to);
  await mkdir(path.dirname(target), { recursive: true });
  await cp(path.join(desktop, from), target, { recursive: true, filter: (source) => !/\/(?:node_modules|\.DS_Store)(?:\/|$)/.test(source) });
}
await writeFile(path.join(outDir, 'build.json'), JSON.stringify({ version: pkg.version, builtAt: new Date().toISOString() }, null, 2));
console.log(`serve host written to ${outDir}`);
