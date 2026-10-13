import { realpathSync } from 'node:fs';
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import os from 'node:os';
import path from 'node:path';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import { bindingIdentity, contractMismatch, findBinding, verifyRuntime } from './runtime-contract';

let dir = '';
beforeEach(async () => { dir = await mkdtemp(path.join(os.tmpdir(), 'pm-contract-')); });
afterEach(async () => { await rm(dir, { recursive: true, force: true }); });

async function fakeBinding(version: string, hash: string): Promise<string> {
  const binding = path.join(dir, 'node_modules', 'transcribe-cpp');
  await mkdir(path.join(binding, 'dist'), { recursive: true });
  await writeFile(path.join(binding, 'package.json'), JSON.stringify({ name: 'transcribe-cpp', version }));
  await writeFile(path.join(binding, 'dist', '_generated.js'), `export const ABI = 1;\nexport const PUBLIC_HEADER_HASH = "${hash}";\n`);
  return binding;
}

async function fakeLibrary(contract: object | null): Promise<string> {
  const lib = path.join(dir, 'unpacked', 'darwin-arm64-metal');
  await mkdir(lib, { recursive: true });
  if (contract) await writeFile(path.join(lib, 'contract.json'), JSON.stringify(contract));
  return path.join(lib, 'libtranscribe.dylib');
}

describe('the transcription runtime contract', () => {
  it('matches on header hash and base version, as the binding does', () => {
    const binding = { version: '0.3.1', headerHash: '57b1af43650f195d' };
    expect(contractMismatch({ version: '0.3.1', header_hash: '57b1af43650f195d' }, binding)).toBeNull();
    expect(contractMismatch({ version: '0.3.1.post2', header_hash: '57b1af43650f195d' }, binding)).toBeNull();
    expect(contractMismatch({ version: '0.3.2', header_hash: '57b1af43650f195d' }, binding)).toMatch(/version 0\.3\.2/);
    expect(contractMismatch({ version: '0.3.1', header_hash: 'ffff' }, binding)).toMatch(/header ffff/);
    expect(contractMismatch({}, binding)).toMatch(/header unknown/);
  });

  it('reads the installed binding and checks the unpacked library against it', async () => {
    const binding = await fakeBinding('0.3.1', 'abc123');
    expect(findBinding([path.join(dir, 'elsewhere'), path.join(dir, 'node_modules')])).toBe(binding);
    expect(findBinding([path.join(dir, 'elsewhere')])).toBeNull();
    expect(await bindingIdentity(binding)).toEqual({ version: '0.3.1', headerHash: 'abc123' });
    await expect(verifyRuntime(await fakeLibrary({ version: '0.3.1', header_hash: 'abc123' }), binding)).resolves.toBeUndefined();
    await expect(verifyRuntime(await fakeLibrary({ version: '0.4.0', header_hash: 'abc123' }), binding)).rejects.toThrow(/Reinstall Powermove/);
  });

  it('refuses a library without its contract', async () => {
    const binding = await fakeBinding('0.3.1', 'abc123');
    await expect(verifyRuntime(await fakeLibrary(null), binding)).rejects.toThrow(/incomplete/);
  });

  it.runIf(process.platform === 'darwin' && process.arch === 'arm64')('agrees with the installed transcribe-cpp and the platform package the app unpacks', async () => {
    const binding = findBinding([path.resolve('node_modules')]);
    expect(binding).not.toBeNull();
    const platform = path.dirname(createRequire(path.join(realpathSync(binding!), 'package.json')).resolve('@transcribe-cpp/darwin-arm64-metal/package.json'));
    await expect(verifyRuntime(path.join(platform, 'libtranscribe.dylib'), binding!)).resolves.toBeUndefined();
  });
});
