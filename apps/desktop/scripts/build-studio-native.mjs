/** Build the renderer and its relocatable runtime. No binary is tracked by Git. */
import { spawnSync } from 'node:child_process';
import { copyFileSync, existsSync, mkdirSync, readdirSync, chmodSync, rmSync } from 'node:fs';
import { homedir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const desktop = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const engine = path.resolve(desktop, '../../engine');
const output = path.join(desktop, 'resources/studio');
const target = process.env.CARGO_TARGET_DIR ?? path.join(homedir(), 'Developer/pm-target');
const cargo = process.env.CARGO ?? (existsSync(path.join(homedir(), '.cargo/bin/cargo')) ? path.join(homedir(), '.cargo/bin/cargo') : 'cargo');
const run = (command, args, options = {}) => {
  const result = spawnSync(command, args, { stdio: 'inherit', ...options });
  if (result.error || result.status !== 0) throw result.error ?? new Error(`${path.basename(command)} failed (${result.status})`);
};
const oidn = process.env.OIDN_DIR ?? '/opt/homebrew/opt/open-image-denoise';
const withOidn = process.platform === 'darwin' && existsSync(path.join(oidn, 'lib/libOpenImageDenoise.dylib'));
if (!withOidn) throw new Error('Native Studio requires Open Image Denoise 2.x (install open-image-denoise or set OIDN_DIR).');
if (process.platform !== 'darwin') throw new Error('Native Studio currently requires macOS; the in-page backend remains portable.');
run('/usr/bin/nice', ['-n', '10', cargo, 'build', '--manifest-path', path.join(engine, 'Cargo.toml'), '--release', '--locked', '-p', 'pm-studio-host', ...(withOidn ? ['--features', 'oidn'] : [])], { env: { ...process.env, CARGO_TARGET_DIR: target, CARGO_BUILD_JOBS: process.env.CARGO_BUILD_JOBS ?? '2' } });
rmSync(output, { recursive: true, force: true });
mkdirSync(output, { recursive: true });
const host = path.join(output, 'pm-studio-host');
copyFileSync(path.join(target, 'release/pm-studio-host'), host); chmodSync(host, 0o755);
const dependencyLines = file => {
  const result = spawnSync('/usr/bin/otool', ['-L', file], { encoding: 'utf8' });
  if (result.status !== 0) throw new Error(`Cannot inspect ${path.basename(file)}`);
  return result.stdout.split('\n').slice(1).map(line => line.trim().split(' (')[0]).filter(Boolean);
};
const runtime = new Map();
const search = [path.join(oidn, 'lib'), '/opt/homebrew/opt/tbb/lib'];
const resolveDependency = (name, from) => {
  if (name.startsWith('/System/') || name.startsWith('/usr/lib/')) return null;
  const choices = name.startsWith('@') ? [path.join(path.dirname(from), path.basename(name)), ...search.map(dir => path.join(dir, path.basename(name)))] : [name];
  const source = choices.find(existsSync);
  if (!source) throw new Error(`Native Studio dependency is missing: ${path.basename(name)}`);
  return source;
};
const collect = source => {
  const filename = path.basename(source);
  if (runtime.has(filename)) return;
  runtime.set(filename, source);
  for (const dependency of dependencyLines(source)) {
    const child = resolveDependency(dependency, source);
    if (child && path.basename(child) !== filename) collect(child);
  }
};
if (withOidn) {
  collect(path.join(oidn, 'lib/libOpenImageDenoise.2.dylib'));
  // OIDN discovers device plugins at runtime, rather than through link commands.
  for (const filename of readdirSync(path.join(oidn, 'lib')).filter(name => /^libOpenImageDenoise_device_.*\.dylib$/.test(name))) collect(path.join(oidn, 'lib', filename));
}
for (const [filename, source] of runtime) {
  const to = path.join(output, filename); copyFileSync(source, to); chmodSync(to, 0o755);
}
for (const [filename, source] of [['pm-studio-host', path.join(target, 'release/pm-studio-host')], ...runtime]) {
  const to = path.join(output, filename);
  for (const dependency of dependencyLines(source)) {
    const resolved = resolveDependency(dependency, source);
    if (resolved && path.basename(resolved) !== filename) run('/usr/bin/install_name_tool', ['-change', dependency, `@rpath/${path.basename(resolved)}`, to]);
  }
  const loads=spawnSync('/usr/bin/otool',['-l',source],{encoding:'utf8'}).stdout;
  const rpaths=[...loads.matchAll(/cmd LC_RPATH\n\s*cmdsize \d+\n\s*path (.+?) \(offset/g)].map(match=>match[1]);
  for(const rpath of rpaths)if(rpath.startsWith('/'))run('/usr/bin/install_name_tool',['-delete_rpath',rpath,to]);
  if (filename.endsWith('.dylib')) run('/usr/bin/install_name_tool', ['-id', `@rpath/${filename}`, to]);
  run('/usr/bin/install_name_tool', ['-add_rpath', filename.endsWith('.dylib') ? '@loader_path' : '@executable_path', to]);
  run('/usr/bin/codesign', ['--force', '--sign', '-', to]);
}
if (withOidn) {
  copyFileSync(path.join(oidn, 'LICENSE.txt'), path.join(output, 'LICENSE-OIDN.txt'));
  const tbbLicense = ['/opt/homebrew/opt/tbb/share/doc/tbb/LICENSE.txt', '/opt/homebrew/opt/tbb/LICENSE.txt'].find(existsSync);
  if (tbbLicense) copyFileSync(tbbLicense, path.join(output, 'LICENSE-TBB.txt'));
}
const nodeInclude = [process.env.NODE_INCLUDE_DIR, path.resolve(path.dirname(process.execPath), '../include/node'), '/opt/homebrew/include/node'].find(dir => dir && existsSync(path.join(dir, 'node_api.h')));
if (!nodeInclude) throw new Error('Node headers are needed to build the Studio surface addon (set NODE_INCLUDE_DIR).');
const addon=path.join(output,'studio-surface.node');
run('xcrun',['clang++','-std=c++17','-fobjc-arc','-mmacosx-version-min=13.0','-O2','-bundle','-undefined','dynamic_lookup','-I',nodeInclude,'-I',path.join(engine,'crates/pm-studio/native'),'-framework','Foundation','-framework','IOSurface',path.join(desktop,'native/studio-surface.mm'),'-o',addon]);
run('/usr/bin/codesign',['--force','--sign','-',addon]);
console.info(`Built native Studio${withOidn ? ' with OIDN' : ''}`);
