import { existsSync } from 'node:fs';
import { readFile } from 'node:fs/promises';
import path from 'node:path';

/*
 * transcribe.cpp's binding checks the native library it loads against the
 * header hash and version it was generated for, but only when it finds the
 * library itself. The packaged app points it at the unpacked copy through
 * TRANSCRIBE_LIBRARY (dlopen cannot map a file inside the asar), which skips
 * that check, so the worker makes it here: a binding and library that drift
 * apart fail with a clear message instead of a crash in native code.
 */

export interface RuntimeContract {
  version?: string;
  header_hash?: string;
}

export interface BindingIdentity {
  version: string;
  headerHash: string;
}

/** '0.3.1' from '0.3.1.post2' or '0.3.1-beta' (the binding's own rule). */
const base = (version: string): string => /^\d+(?:\.\d+)*/.exec(version.trim())?.[0] ?? version.trim();

/** Why a library cannot be loaded by this binding, or null when it can. */
export function contractMismatch(contract: RuntimeContract, binding: BindingIdentity): string | null {
  if (contract.header_hash !== binding.headerHash) {
    return `the transcription runtime was built for header ${contract.header_hash ?? 'unknown'}, but its binding expects ${binding.headerHash}`;
  }
  if (contract.version && base(contract.version) !== base(binding.version)) {
    return `the transcription runtime is version ${contract.version}, but its binding is ${binding.version}`;
  }
  return null;
}

/** The installed binding's version and header hash, read from its package. */
export async function bindingIdentity(dir: string): Promise<BindingIdentity> {
  const { version } = JSON.parse(await readFile(path.join(dir, 'package.json'), 'utf8')) as { version?: string };
  const generated = await readFile(path.join(dir, 'dist', '_generated.js'), 'utf8');
  const headerHash = /PUBLIC_HEADER_HASH\s*=\s*["']([0-9a-f]+)["']/.exec(generated)?.[1];
  if (!version || !headerHash) throw new Error('The transcription binding does not say which runtime it needs.');
  return { version, headerHash };
}

/** The transcribe-cpp package folder on a module search path (it exports no CommonJS entry to resolve). */
export function findBinding(searchPaths: readonly string[]): string | null {
  for (const dir of searchPaths) {
    const candidate = path.join(dir, 'transcribe-cpp');
    if (existsSync(path.join(candidate, 'package.json'))) return candidate;
  }
  return null;
}

/** Throws when the library next to its contract.json does not match the binding. */
export async function verifyRuntime(library: string, bindingDir: string): Promise<void> {
  let contract: RuntimeContract;
  try {
    contract = JSON.parse(await readFile(path.join(path.dirname(library), 'contract.json'), 'utf8')) as RuntimeContract;
  } catch {
    throw new Error('The transcription runtime is incomplete. Reinstall Powermove.');
  }
  const problem = contractMismatch(contract, await bindingIdentity(bindingDir));
  if (problem) throw new Error(`Transcription can’t start: ${problem}. Reinstall Powermove.`);
}
