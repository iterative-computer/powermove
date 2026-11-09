/** Reject an incomplete, non-relocatable or inconsistently signed Studio bundle. */
import { spawnSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { readdir, access } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

/** Exercise the real binary, not just its filename or Cargo's cache result. */
export function verifyStudioHost(executable) {
  const desktop = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
  const materialFloats = /export const STUDIO_MATERIAL_FLOATS\s*=\s*(\d+)/.exec(readFileSync(path.join(desktop, 'src/renderer/src/core/studio/extract.ts'), 'utf8'))?.[1];
  if (!materialFloats) throw new Error('Cannot determine the Studio renderer layout.');
  const json = Buffer.from(JSON.stringify({id:1,op:'hello'})), header = Buffer.alloc(12);
  header.write('PMS1');header.writeUInt32LE(json.length,4);
  const reply = spawnSync(executable, [], {input:Buffer.concat([header,json]),timeout:10_000,maxBuffer:1024*1024});
  if (reply.error || reply.status !== 0 || !reply.stdout || reply.stdout.length < 12 || reply.stdout.toString('ascii',0,4) !== 'PMS1') throw new Error('Built Studio host did not complete its compatibility handshake.');
  const size = reply.stdout.readUInt32LE(4);
  if (size > reply.stdout.length - 12 || reply.stdout.readUInt32LE(8) !== 0) throw new Error('Built Studio host returned an invalid handshake.');
  const result = JSON.parse(reply.stdout.toString('utf8',12,12+size));
  if (result.id !== 1 || !result.ok || result.result?.protocol !== 1 || result.result.layout?.materialBytes !== Number(materialFloats)*4 || result.result.layout?.triangleBytes !== 96 || result.result.layout?.nodeBytes !== 64 || result.result.layout?.instanceFloats !== 36) throw new Error('Built Studio host does not match the renderer layout; rebuild in the checkout-specific Cargo target.');
}

function inspect(command, args) {
  const result = spawnSync(command, args, { encoding: 'utf8' });
  if (result.error || result.status !== 0) throw result.error ?? new Error(`${path.basename(command)} failed (${result.status})`);
  return result.stdout + result.stderr;
}
function signature(file) {
  const details = inspect('/usr/bin/codesign', ['-dv', '--verbose=2', file]);
  return { team: /TeamIdentifier=(.+)/.exec(details)?.[1]?.trim(), runtime: /flags=0x[0-9a-f]+\([^)]*runtime[^)]*\)/.test(details) };
}
export async function verifyStudioRuntime(app, { adHoc = false } = {}) {
  const root = path.join(app, 'Contents/Resources/studio');
  for (const name of ['pm-studio-host', 'studio-surface.node', 'libOpenImageDenoise.2.dylib', 'LICENSE-OIDN.txt', 'LICENSE-TBB.txt', 'LICENSE-ufbx.txt']) await access(path.join(root, name));
  const files = (await readdir(root)).filter(name => name === 'pm-studio-host' || /\.(dylib|node)$/.test(name));
  if (!files.some(name => name.startsWith('libOpenImageDenoise_device_cpu.')) || !files.some(name => /^libtbb\./.test(name))) throw new Error('Studio CPU denoiser or TBB runtime is missing.');
  const appSignature = signature(app);
  if (!appSignature.runtime || !appSignature.team || (!adHoc && appSignature.team === 'not set')) throw new Error('The app needs a consistent hardened signature.');
  for (const name of files) {
    const file = path.join(root, name), info = signature(file);
    if (info.team !== appSignature.team || !info.runtime) throw new Error(`${name} does not match the app's hardened signing policy.`);
    inspect('/usr/bin/codesign', ['--verify', '--strict', file]);
    const loads = inspect('/usr/bin/otool', ['-L', file]).split('\n').slice(1).map(line => line.trim().split(' (')[0]).filter(Boolean);
    if (loads.some(load => load.startsWith('/') && !load.startsWith('/System/') && !load.startsWith('/usr/lib/'))) throw new Error(`${name} has a non-relocatable library dependency.`);
    const rpaths = [...inspect('/usr/bin/otool', ['-l', file]).matchAll(/cmd LC_RPATH\n\s*cmdsize \d+\n\s*path (.+?) \(offset/g)].map(match => match[1]);
    if (rpaths.some(value => value.startsWith('/'))) throw new Error(`${name} has an absolute runtime search path.`);
  }
  verifyStudioHost(path.join(root,'pm-studio-host'));
  return files;
}
