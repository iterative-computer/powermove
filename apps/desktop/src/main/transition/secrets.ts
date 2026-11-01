import { createHash } from 'node:crypto';
import { constants } from 'node:fs';
import { open, readdir, readFile } from 'node:fs/promises';
import path from 'node:path';
import { decodeStored, parseEnvText, type SafeStorageLike } from '../env/store';
import { writeJSON } from './json';

/** Implemented by the signed macOS Security.framework adapter. Never by a renderer.
 * SecAccess must contain exactly these two pinned designated requirements.
 * Keychain performs encryption; decrypted bytes never go to a profile file. */
export interface LegacyKeychain {
  putLegacyItem(input: { service: 'Powermove Native Legacy Handoff'; account: string; value: Uint8Array;
    designatedRequirements: readonly [string, string] }): Promise<{ persistentRef: string; designatedRequirements: readonly string[] }>;
}
export type SealedSecret = { locator: string; sealed: Buffer };
const sha = (value: string | Uint8Array) => createHash('sha256').update(value).digest('hex');
async function regularFile(file: string): Promise<Buffer> {
  const handle = await open(file, constants.O_RDONLY | constants.O_NOFOLLOW);
  try {
    const info = await handle.stat();
    if (!info.isFile() || info.size > 32 * 1024 ** 2) throw new Error('Invalid sealed secret file.');
    return await handle.readFile();
  } finally { await handle.close(); }
}
async function names(dir: string): Promise<string[]> {
  try { return await readdir(dir); } catch (error: any) { if (error.code === 'ENOENT') return []; throw error; }
}
/** All four sealed families in D-STATE, including every saved provider address. */
export async function discoverSecrets(profile: string): Promise<SealedSecret[]> {
  const output: SealedSecret[] = [];
  const binary = async (relative: string) => {
    try { output.push({ locator: relative, sealed: await regularFile(path.join(profile, relative)) }); }
    catch (error: any) { if (error.code !== 'ENOENT') throw error; }
  };
  await binary('cloud/session.bin');
  for (const name of await names(path.join(profile, 'cloud/human'))) if (name.endsWith('.bin')) await binary(`cloud/human/${name}`);
  for (const name of await names(path.join(profile, 'env'))) {
    if (!name.endsWith('.env')) continue;
    const lines = parseEnvText((await regularFile(path.join(profile, 'env', name))).toString('utf8'));
    for (const line of lines) if (line.kind === 'var') {
      const value = decodeStored(line.stored);
      if (value.sealed) output.push({ locator: `env/${name}:${line.key}`, sealed: Buffer.from(value.payload, 'base64') });
    }
  }
  try {
    const provider = JSON.parse((await regularFile(path.join(profile, 'agent-provider.json'))).toString('utf8'));
    if (typeof provider.secret === 'string') output.push({ locator: 'agent-provider.json:secret', sealed: Buffer.from(provider.secret, 'base64') });
    if (provider.secrets && typeof provider.secrets === 'object') for (const [address, value] of Object.entries(provider.secrets)) {
      if (typeof value !== 'string') throw new Error('Invalid sealed provider record.');
      output.push({ locator: `agent-provider.json:secrets:${address}`, sealed: Buffer.from(value, 'base64') });
    }
  } catch (error: any) { if (error.code !== 'ENOENT') throw error; }
  return output.sort((a, b) => a.locator.localeCompare(b.locator));
}
export class SecretHandoff {
  constructor(private options: { profile: string; safeStorage: SafeStorageLike; keychain?: LegacyKeychain;
    designatedRequirements?: readonly [string, string] }) {}
  async run(): Promise<void> {
    const file = path.join(this.options.profile, 'Media Store', 'secret-handoff.json');
    const secrets = await discoverSecrets(this.options.profile);
    const requirements = this.options.designatedRequirements;
    if (secrets.length && (!this.options.keychain || !requirements || requirements[0] === requirements[1]
      || requirements.some(x => !x.includes('anchor apple generic') || !x.includes('identifier') || !x.includes('certificate')))) {
      throw new Error('Sign-in transfer needs the signed macOS Keychain adapter and both verified app identities.');
    }
    if (secrets.length && !this.options.safeStorage.isEncryptionAvailable()) throw new Error('Unlock your Mac to prepare sign-ins.');
    let receipts: Record<string, { sourceHash: string; persistentRef: string; requirementsHash: string }> = {};
    try {
      const checkpoint = JSON.parse(await readFile(file, 'utf8'));
      if (checkpoint.version !== 1 || !checkpoint.receipts || typeof checkpoint.receipts !== 'object' || Array.isArray(checkpoint.receipts)) throw new Error('Invalid receipt checkpoint.');
      receipts = checkpoint.receipts;
    }
    catch (error: any) { if (error.code !== 'ENOENT') throw new Error('The sign-in transfer checkpoint needs recovery.'); }
    const before = sha(JSON.stringify(secrets.map(s => [s.locator, sha(s.sealed)])));
    for (const secret of secrets) {
      const sourceHash = sha(secret.sealed), requirementsHash = sha(JSON.stringify(requirements));
      const account = sha(JSON.stringify([secret.locator, sourceHash, requirementsHash]));
      if (receipts[account]?.sourceHash === sourceHash && receipts[account]?.requirementsHash === requirementsHash) continue;
      let plaintext: Buffer | undefined;
      try {
        // Electron decryptString necessarily creates a short-lived JS string.
        plaintext = Buffer.from(this.options.safeStorage.decryptString(secret.sealed), 'utf8');
        const receipt = await this.options.keychain!.putLegacyItem({ service: 'Powermove Native Legacy Handoff', account,
          value: plaintext, designatedRequirements: requirements! });
        if (!receipt.persistentRef || JSON.stringify([...receipt.designatedRequirements].sort()) !== JSON.stringify([...requirements!].sort())) throw new Error('Invalid Keychain ACL receipt.');
        receipts[account] = { sourceHash, persistentRef: receipt.persistentRef, requirementsHash };
        await writeJSON(file, { version: 1, state: 'copying', receipts });
      } catch { throw new Error('Sign-in transfer was not verified. The original encrypted files have been kept.'); }
      finally { plaintext?.fill(0); }
    }
    const after = await discoverSecrets(this.options.profile);
    if (sha(JSON.stringify(after.map(s => [s.locator, sha(s.sealed)]))) !== before) throw new Error('Sign-ins changed during preparation. Retry to transfer the latest values.');
    await writeJSON(file, { version: 1, state: 'complete', receipts });
  }
}
