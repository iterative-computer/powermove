// Wraps sindresorhus/create-dmg: a plain Finder window with the app on the
// left and Applications on the right, no background art. electron-builder
// keeps producing the ZIP that electron-updater needs; the DMG is only for
// humans, so it comes from here. Run after `electron-builder --mac`.
import { execFile } from 'node:child_process';
import { access, readdir, readFile, rename, rm } from 'node:fs/promises';
import { createRequire } from 'node:module';
import path from 'node:path';
import { promisify } from 'node:util';

const execFileAsync = promisify(execFile);
const repository = path.resolve(import.meta.dirname, '..');
const dist = path.join(repository, 'dist');

async function findPackagedApp() {
  for (const entry of await readdir(dist, { withFileTypes: true })) {
    if (!entry.isDirectory() || !entry.name.startsWith('mac')) continue;
    const candidate = path.join(dist, entry.name, 'Powermove.app');
    try { await readdir(candidate); return { app: candidate, arch: entry.name.replace(/^mac-?/, '') || 'x64' }; } catch { /* next */ }
  }
  throw new Error('Powermove.app was not found in dist/. Run electron-builder --mac first.');
}

// bun does not run node-gyp for packages that ship only a binding.gyp (npm
// does). Two of create-dmg's dependencies are like that; build them when
// missing. Resolution follows the real dependency chain because bun isolates
// nested node_modules.
async function ensureNativeAddons() {
  const from = (base, id) => path.dirname(createRequire(path.join(base, 'package.json')).resolve(`${id}/package.json`));
  const createDmg = from(repository, 'create-dmg');
  const appdmg = from(createDmg, 'appdmg');
  const addons = [
    [from(appdmg, 'fs-xattr'), 'xattr.node'],
    [from(from(appdmg, 'ds-store'), 'macos-alias'), 'volume.node']
  ];
  for (const [pkg, artifact] of addons) {
    try { await access(path.join(pkg, 'build/Release', artifact)); continue; } catch { /* build */ }
    process.stdout.write(`Building ${path.basename(pkg)} native addon…\n`);
    await execFileAsync('npx', ['--yes', 'node-gyp', 'rebuild'], { cwd: pkg, maxBuffer: 20 * 1024 * 1024 });
  }
}

async function main() {
  await ensureNativeAddons();
  const { version } = JSON.parse(await readFile(path.join(repository, 'package.json'), 'utf8'));
  const { app, arch } = await findPackagedApp();
  const target = path.join(dist, `Powermove-${version}-mac-${arch}.dmg`);
  await rm(target, { force: true });
  await rm(`${target}.blockmap`, { force: true });

  const bin = path.join(repository, 'node_modules/.bin/create-dmg');
  // create-dmg signs with the Developer ID it finds in the keychain search
  // list and exits 2 when there is none. Local builds may go unsigned; CI
  // may not.
  try {
    const { stdout, stderr } = await execFileAsync(bin, [app, dist, '--overwrite'], { cwd: repository, maxBuffer: 20 * 1024 * 1024 });
    process.stdout.write(stdout); process.stderr.write(stderr);
  } catch (error) {
    if (error.code !== 2 || process.env.CI) throw error;
    process.stderr.write(`${String(error.stderr ?? '').trim()}\nDMG left unsigned (no Developer ID in the keychain).\n`);
  }

  // create-dmg names the image "<Product> <version>.dmg" from the bundle.
  const produced = (await readdir(dist)).find((name) => /^Powermove .*\.dmg$/.test(name));
  if (!produced) throw new Error('create-dmg did not produce a DMG.');
  await rename(path.join(dist, produced), target);
  process.stdout.write(`DMG: ${target}\n`);
}

main().catch((error) => {
  process.stderr.write(`${error instanceof Error ? error.message : String(error)}\n`);
  process.exitCode = 1;
});
