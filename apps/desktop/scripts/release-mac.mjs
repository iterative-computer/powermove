import { execFile } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync } from 'node:fs';
import { readdir, readFile, stat } from 'node:fs/promises';
import path from 'node:path';
import { promisify } from 'node:util';

const execFileAsync = promisify(execFile);
const repository = path.resolve(import.meta.dirname, '..');

function releaseCredentials(environment) {
  const missing = [];
  const importedCertificate = environment.CSC_NAME && environment.CSC_KEYCHAIN;
  if (!importedCertificate) {
    if (!environment.CSC_LINK) missing.push('CSC_LINK (or CSC_NAME + CSC_KEYCHAIN)');
    if (!environment.CSC_KEY_PASSWORD) missing.push('CSC_KEY_PASSWORD (or CSC_NAME + CSC_KEYCHAIN)');
  }
  const apiKey = environment.APPLE_API_KEY && environment.APPLE_API_KEY_ID && environment.APPLE_API_ISSUER;
  const appleId = environment.APPLE_ID && environment.APPLE_APP_SPECIFIC_PASSWORD && environment.APPLE_TEAM_ID;
  if (!apiKey && !appleId) {
    missing.push('APPLE_API_KEY + APPLE_API_KEY_ID + APPLE_API_ISSUER (or Apple ID notarization credentials)');
  }
  return missing;
}

async function run(command, args) {
  try {
    const { stdout, stderr } = await execFileAsync(command, args, {
      cwd: repository,
      env: process.env,
      maxBuffer: 20 * 1024 * 1024,
    });
    if (stdout) process.stdout.write(stdout);
    if (stderr) process.stderr.write(stderr);
  } catch (error) {
    if (error?.stdout) process.stdout.write(error.stdout);
    if (error?.stderr) process.stderr.write(error.stderr);
    throw error;
  }
}

async function findPackagedApp() {
  const dist = path.join(repository, 'dist');
  for (const entry of await readdir(dist, { withFileTypes: true })) {
    if (!entry.isDirectory() || !entry.name.startsWith('mac')) continue;
    const candidate = path.join(dist, entry.name, 'Powermove.app');
    try {
      await readdir(candidate);
      return candidate;
    } catch {
      // Keep looking for electron-builder's architecture-specific folder.
    }
  }
  throw new Error('The signed Powermove.app was not found in dist/.');
}

async function signingInfo(file) {
  // codesign prints its details on stderr.
  const { stderr } = await execFileAsync('/usr/bin/codesign', ['-dv', '--verbose=2', file]);
  return {
    team: /TeamIdentifier=(.+)/.exec(stderr)?.[1]?.trim() ?? 'not set',
    runtime: /flags=0x[0-9a-f]+\([^)]*runtime[^)]*\)/.test(stderr)
  };
}

/* Native code that main or a utility process loads from app.asar.unpacked
   must carry the app's own Team ID and the hardened runtime, or library
   validation refuses it at load time on users' machines:
   - the transcription utility process loads koffi's addon, which dlopens
     transcribe.cpp's libtranscribe and ggml dylibs;
   - the native engine's utility processes load pm-node (native/pm_node.node)
     and main loads pm-bridge for the frame bridge (native/pm_bridge.node),
     both built by `bun run build:engine` and shipped with --with-engine.
   Every .node and .dylib anywhere under app.asar.unpacked is checked, not
   only these; the list says which must be present. */
const NATIVE_RUNTIME = [
  { dir: 'node_modules/@transcribe-cpp/darwin-arm64-metal', expect: ['libtranscribe.dylib', 'libggml.dylib', 'libggml-base.dylib', 'libggml-cpu.dylib', 'libggml-metal.dylib'] },
  { dir: 'node_modules/@koromix/koffi-darwin-arm64/darwin_arm64', expect: ['koffi.node'] }
];
const ENGINE_RUNTIME = { dir: 'native', expect: ['pm_node.node', 'pm_bridge.node'] };
/* The evaluator module for the renderer and the headless document host
   (engine/crates/pm-wasm/scripts/build.mjs writes it; electron-vite copies
   it to out/renderer/engine/, which app.asar packs). */
const WASM_SOURCE = path.join(repository, 'src/renderer/public/engine/pm_wasm.wasm');
const WASM_PACKED = 'out/renderer/engine/pm_wasm.wasm';

async function sha256(file) {
  return createHash('sha256').update(await readFile(file)).digest('hex');
}

async function buildWasm() {
  const before = existsSync(WASM_SOURCE) ? (await stat(WASM_SOURCE)).mtimeMs : -1;
  await run('node', [path.resolve(repository, '../../engine/crates/pm-wasm/scripts/build.mjs')]);
  if (!existsSync(WASM_SOURCE) || (await stat(WASM_SOURCE)).mtimeMs === before) {
    throw new Error(`build:wasm did not write ${path.relative(repository, WASM_SOURCE)}.`);
  }
  return sha256(WASM_SOURCE);
}

/** A file's bytes from an asar archive (Chromium pickle header, then JSON). */
export async function readAsarFile(archive, relative) {
  const bytes = await readFile(archive);
  const headerSize = bytes.readUInt32LE(4);
  const header = JSON.parse(bytes.subarray(16, 16 + bytes.readUInt32LE(12)).toString('utf8'));
  let node = header;
  for (const part of relative.split('/')) {
    node = node?.files?.[part];
    if (!node) throw new Error(`${relative} is not in ${archive}.`);
  }
  if (node.unpacked || node.offset === undefined) throw new Error(`${relative} is not packed in ${archive}.`);
  const start = 8 + headerSize + Number(node.offset);
  return bytes.subarray(start, start + node.size);
}

/* The shipped engine must be the one just built: both addons export their
   Node-API entry point (an LTO link once produced a 471 KB addon that loaded
   nothing, docs/engine/release-build.md), and app.asar carries the fresh
   evaluator module. */
async function verifyEngine(app, wasmHash) {
  const unpacked = path.join(app, 'Contents/Resources/app.asar.unpacked', ENGINE_RUNTIME.dir);
  for (const name of ENGINE_RUNTIME.expect) {
    const { stdout } = await execFileAsync('/usr/bin/nm', ['-gU', path.join(unpacked, name)], { maxBuffer: 64 * 1024 * 1024 });
    if (!/\bnapi_register_module_v1\b/.test(stdout)) throw new Error(`${name} does not export napi_register_module_v1.`);
  }
  const packed = createHash('sha256').update(await readAsarFile(path.join(app, 'Contents/Resources/app.asar'), WASM_PACKED)).digest('hex');
  if (packed !== wasmHash) throw new Error(`app.asar's ${WASM_PACKED} (${packed}) is not the module just built (${wasmHash}).`);
  process.stdout.write(`Engine addons export napi_register_module_v1; app.asar carries pm_wasm.wasm ${wasmHash}.\n`);
}

async function nativeBinaries(root) {
  const found = [];
  for (const entry of await readdir(root, { withFileTypes: true, recursive: true })) {
    if (entry.isFile() && (entry.name.endsWith('.node') || entry.name.endsWith('.dylib'))) {
      found.push(path.join(entry.parentPath ?? entry.path, entry.name));
    }
  }
  return found.sort();
}

/* The packaged worker points transcribe-cpp at the unpacked library through
   TRANSCRIBE_LIBRARY, which skips the binding's own contract check (the
   worker repeats it at runtime, runtime-contract.ts). Fail the release
   rather than ship a library its binding does not match. */
async function verifyRuntimeContract(app) {
  const binding = path.join(repository, 'node_modules/transcribe-cpp');
  const { version } = JSON.parse(await readFile(path.join(binding, 'package.json'), 'utf8'));
  const headerHash = /PUBLIC_HEADER_HASH\s*=\s*["']([0-9a-f]+)["']/.exec(await readFile(path.join(binding, 'dist/_generated.js'), 'utf8'))?.[1];
  const contract = JSON.parse(await readFile(path.join(app, 'Contents/Resources/app.asar.unpacked', NATIVE_RUNTIME[0].dir, 'contract.json'), 'utf8'));
  const base = (value) => /^\d+(?:\.\d+)*/.exec(String(value).trim())?.[0];
  if (!headerHash || contract.header_hash !== headerHash || base(contract.version) !== base(version)) {
    throw new Error(`The unpacked transcription runtime (${contract.version}, header ${contract.header_hash}) does not match transcribe-cpp ${version} (header ${headerHash}).`);
  }
  process.stdout.write(`Transcription runtime ${contract.version} matches its binding (header ${headerHash}).\n`);
}

/* Powermove's FFmpeg executables load their libav* dylibs from
   Resources/ffmpeg/lib. Under the hardened runtime every one must carry the
   app's Team ID, or export, proxies, transcription and the agent media tools
   fail on users' machines. Run the signed ffmpeg once to prove it loads. */
async function verifyFfmpeg(app) {
  const appInfo = await signingInfo(app);
  const root = path.join(app, 'Contents/Resources/ffmpeg');
  const files = [
    ...(await readdir(path.join(root, 'bin'))).map((name) => path.join(root, 'bin', name)),
    ...(await readdir(path.join(root, 'lib'))).filter((name) => name.endsWith('.dylib')).map((name) => path.join(root, 'lib', name))
  ];
  for (const name of ['bin/ffmpeg', 'bin/ffprobe']) {
    if (!files.includes(path.join(root, name))) throw new Error(`FFmpeg ${name} is missing from ${root}.`);
  }
  for (const file of files) {
    const info = await signingInfo(file);
    if (info.team !== appInfo.team || !info.runtime) {
      throw new Error(`${path.relative(root, file)} is not signed for the hardened runtime (team ${info.team}, app team ${appInfo.team}, runtime ${info.runtime}).`);
    }
  }
  const license = await execFileAsync(path.join(root, 'bin/ffmpeg'), ['-hide_banner', '-L']);
  if (!/GNU Lesser General Public/.test(license.stdout + license.stderr)) throw new Error('The packaged ffmpeg does not report the LGPL.');
  process.stdout.write(`FFmpeg signed with team ${appInfo.team}: ${files.map((file) => path.relative(root, file)).join(', ')}\n`);
}

async function verifyNativeAddons(app, { withEngine }) {
  await verifyRuntimeContract(app);
  const appInfo = await signingInfo(app);
  const unpacked = path.join(app, 'Contents/Resources/app.asar.unpacked');
  const required = withEngine ? [...NATIVE_RUNTIME, ENGINE_RUNTIME] : NATIVE_RUNTIME;
  for (const { dir: relative, expect } of required) {
    const dir = path.join(unpacked, relative);
    const names = await readdir(dir).catch(() => []);
    const missing = expect.filter((name) => !names.includes(name));
    if (missing.length) throw new Error(`Native binaries missing from ${dir}: ${missing.join(', ')}.`);
  }
  const signed = [];
  for (const file of await nativeBinaries(unpacked)) {
    const info = await signingInfo(file);
    if (info.team !== appInfo.team || !info.runtime) {
      throw new Error(`${path.relative(unpacked, file)} is not signed for library validation (team ${info.team}, app team ${appInfo.team}, runtime ${info.runtime}).`);
    }
    signed.push(path.relative(unpacked, file));
  }
  process.stdout.write(`Native binaries signed with team ${appInfo.team} and the hardened runtime: ${signed.join(', ')}\n`);
}

async function main() {
  const missing = releaseCredentials(process.env);
  if (missing.length) {
    throw new Error(`Production release credentials are incomplete: ${missing.join(', ')}.`);
  }
  if (process.argv.includes('--check')) {
    process.stdout.write('Production signing and notarization credentials are present.\n');
    return;
  }

  // electron-builder names the update manifest after publish.channel and
  // defaults to latest-mac.yml. electron-updater asks for beta-mac.yml on
  // prerelease versions (falling back to latest), so keep the two in step.
  const { version } = JSON.parse(await readFile(path.join(repository, 'package.json'), 'utf8'));
  const channel = /^\d+\.\d+\.\d+-([a-z]+)\./i.exec(version)?.[1]?.toLowerCase() ?? 'latest';

  // The native engine ships only when asked: it is behind "Use New Engine"
  // and needs the pinned Rust toolchain (engine/env.sh) on the build host.
  const withEngine = process.argv.includes('--with-engine');
  for (const file of ENGINE_RUNTIME.expect) {
    if (!withEngine && existsSync(path.join(repository, ENGINE_RUNTIME.dir, file))) {
      throw new Error(`native/${file} is present but --with-engine was not given: electron-builder would ship it. Remove native/ or pass --with-engine.`);
    }
  }
  if (!withEngine && existsSync(WASM_SOURCE)) {
    throw new Error(`${path.relative(repository, WASM_SOURCE)} is present but --with-engine was not given: the renderer build would ship it. Remove it or pass --with-engine.`);
  }
  await run('node', [path.join(repository, 'scripts/clean-renderer.mjs')]);
  // pm_wasm.wasm is gitignored and copied into the renderer bundle as is, so
  // build it here: a release must never ship whatever build:wasm last wrote.
  const wasmHash = withEngine ? await buildWasm() : null;
  await run('bun', ['run', 'build']);
  if (withEngine) {
    const bundled = await sha256(path.join(repository, 'out/renderer/engine/pm_wasm.wasm'));
    if (bundled !== wasmHash) throw new Error(`out/renderer/engine/pm_wasm.wasm (${bundled}) is not the module just built (${wasmHash}).`);
    await run('node', [path.join(repository, 'scripts/build-engine-addon.mjs')]);
  }
  // Stage Powermove's FFmpeg from its build (as dist:mac does) rather than
  // packaging whatever vendor/ffmpeg held; package.sh verifies it first.
  await run('bun', ['run', 'stage:ffmpeg']);
  await run(path.join(repository, 'node_modules/.bin/electron-builder'), [
    '--mac', '--arm64', '--publish', 'never',
    '--config', 'electron-builder.yml',
    `--config.publish.channel=${channel}`,
    '--config.mac.identity=Developer ID Application',
    '--config.mac.hardenedRuntime=true',
    '--config.mac.notarize=true',
  ]);

  await run('node', [path.join(repository, 'scripts/create-dmg.mjs')]);

  const app = await findPackagedApp();
  await run('/usr/bin/codesign', ['--verify', '--deep', '--strict', '--verbose=2', app]);
  await verifyNativeAddons(app, { withEngine });
  if (withEngine) await verifyEngine(app, wasmHash);
  await verifyFfmpeg(app);
  await run('/usr/sbin/spctl', ['--assess', '--type', 'execute', '--verbose=4', app]);
  await run('/usr/bin/xcrun', ['stapler', 'validate', app]);

  const dist = path.join(repository, 'dist');
  for (const entry of await readdir(dist, { withFileTypes: true })) {
    if (!entry.isFile() || !entry.name.endsWith('.dmg')) continue;
    const artifact = path.join(dist, entry.name);
    await run('/usr/bin/hdiutil', ['verify', artifact]);
    // electron-builder notarizes/staples the app before creating its archives.
    // The enclosing DMG has no separate notarization ticket.
  }
  process.stdout.write(`Verified signed and notarized release: ${app}\n`);
}

if (process.argv[1] && path.resolve(process.argv[1]) === import.meta.filename) main().catch((error) => {
  process.stderr.write(`${error instanceof Error ? error.message : String(error)}\n`);
  process.exitCode = 1;
});
