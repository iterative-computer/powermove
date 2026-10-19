/** Assemble already-built native CLI artifacts; never builds, tags or publishes.
 * node scripts/assemble-native.mjs --arch arm64 --artifacts <cargo profile dir>
 * --ffmpeg <LGPL build out> [--out <native root>] [--node <clean Node 22>]
 */
import { execFileSync } from 'node:child_process';
import { copyFileSync, cpSync, existsSync, mkdirSync, mkdtempSync, realpathSync, renameSync, rmSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const args = process.argv.slice(2), flags = new Map();
while (args.length) {
  const flag = args.shift(), value = args.shift();
  if (!['--arch', '--artifacts', '--ffmpeg', '--out', '--node'].includes(flag) || !value) throw new Error(`Invalid assembly option ${flag}`);
  flags.set(flag, value);
}
const arch = flags.get('--arch');
if (!['arm64', 'x64'].includes(arch)) throw new Error('--arch must be arm64 or x64');
const artifacts = path.resolve(flags.get('--artifacts') ?? ''), ffmpeg = path.resolve(flags.get('--ffmpeg') ?? process.env.PM_FFMPEG_DIR ?? '');
if (!flags.has('--artifacts') || !existsSync(path.join(ffmpeg, 'manifest.json'))) throw new Error('--artifacts and a verified LGPL --ffmpeg build are required');
const root = path.resolve(flags.get('--out') ?? fileURLToPath(new URL('../prebuilds', import.meta.url)));
mkdirSync(root, { recursive: true });
const stage = mkdtempSync(path.join(root, `.darwin-${arch}-`)), lib = path.join(stage, 'lib');
mkdirSync(lib);
const run = (file, argv) => execFileSync(file, argv, { encoding: 'utf8' });
const copied = new Map();
const cpu = arch === 'x64' ? 'x86_64' : 'arm64';
function copyBinary(source, target) {
  source = realpathSync(source);
  if (!run('lipo', ['-archs', source]).trim().split(/\s+/).includes(cpu)) throw new Error(`native-architecture-mismatch: ${source} does not contain ${cpu}`);
  copyFileSync(source, target);
  return source;
}
function dependencies(source, target) {
  const lines = run('otool', ['-L', source]).split('\n').slice(1);
  for (const line of lines) {
    const dependency = line.trim().split(' (')[0];
    if (!dependency || dependency.startsWith('/System/') || dependency.startsWith('/usr/lib/') || dependency === source || dependency === run('otool', ['-D', source]).split('\n')[1]?.trim()) continue;
    const name = path.basename(dependency);
    const from = dependency.startsWith('@rpath/') ? path.join(ffmpeg, 'lib', name) : dependency.startsWith('@loader_path/') ? path.resolve(path.dirname(source), dependency.slice(13)) : dependency;
    const resolved = realpathSync(from);
    if (copied.has(name) && copied.get(name) !== resolved) throw new Error(`native-library-collision: ${name}`);
    if (!copied.has(name)) {
      copied.set(name, resolved);
      const dest = path.join(lib, name);
      copyBinary(resolved, dest);
      run('install_name_tool', ['-id', `@rpath/${name}`, dest]);
      dependencies(resolved, dest);
    }
    const relative = path.dirname(target) === lib ? `@loader_path/${name}` : `@loader_path/lib/${name}`;
    run('install_name_tool', ['-change', dependency, relative, target]);
  }
}
try {
  for (const [from, to] of [['libpm_node.dylib', 'pm_node.node'], ['libpm_bridge.dylib', 'pm_bridge.node'], ['pm-cli', 'pm-cli']]) {
    const source = copyBinary(path.join(artifacts, from), path.join(stage, to));
    dependencies(source, path.join(stage, to));
  }
  cpSync(path.join(ffmpeg, 'share/licenses'), path.join(stage, 'share/licenses'), { recursive: true });
  copyFileSync(path.join(ffmpeg, 'manifest.json'), path.join(stage, 'ffmpeg-manifest.json'));
  for (const file of [...copied.keys()].map(name => path.join(lib, name)).concat(['pm_node.node', 'pm_bridge.node', 'pm-cli'].map(name => path.join(stage, name)))) run('codesign', ['--force', '--sign', '-', file]);
  const node = flags.get('--node') ?? process.execPath;
  if (Number(run(node, ['-p', 'process.versions.node.split(".")[0]']).trim()) !== 22) throw new Error('--node must name a clean Node 22 executable for the required load test');
  run(node, ['-e', 'const a = require(process.argv[1]); const b = require(process.argv[2]); if (!a.buildId() || typeof a.EngineHost !== "function" || typeof b.FontRegistry !== "function") process.exit(1); const engine = new a.EngineHost("job"); engine.tick(); engine.pump();', path.join(stage, 'pm_node.node'), path.join(stage, 'pm_bridge.node')]);
  run(path.join(stage, 'pm-cli'), ['--help']);
  const destination = path.join(root, `darwin-${arch}`);
  if (existsSync(destination)) throw new Error(`native-output-exists: ${destination}; select a fresh --out root`);
  renameSync(stage, destination);
  process.stdout.write(`${destination}: Node 22 load and pm-cli launch passed\n`);
} finally { rmSync(stage, { recursive: true, force: true }); }
