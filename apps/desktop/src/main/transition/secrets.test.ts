import { mkdir, mkdtemp, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { discoverSecrets, SecretHandoff, type LegacyKeychain } from './secrets';
import { signedKeychain } from './keychain';
let profile: string;
const safeStorage = { isEncryptionAvailable: () => true, decryptString: (b: Buffer) => b.toString().slice(7), encryptString: (v: string) => Buffer.from(`sealed:${v}`) };
const requirements: [string, string] = ['identifier "synthetic.electron" and anchor apple generic and certificate leaf[subject.OU] = "SYNTHETIC"', 'identifier "synthetic.native" and anchor apple generic and certificate leaf[subject.OU] = "SYNTHETIC"'];
beforeEach(async () => { profile = await mkdtemp(path.join(os.tmpdir(), 'pm-tr-secrets-')); });
afterEach(async () => { await rm(profile, { recursive: true, force: true }); });
async function fixture() {
  await mkdir(path.join(profile, 'cloud/human'), { recursive: true }); await mkdir(path.join(profile, 'env'));
  await writeFile(path.join(profile, 'cloud/session.bin'), safeStorage.encryptString('synthetic-session'));
  await writeFile(path.join(profile, 'cloud/human/proof.bin'), safeStorage.encryptString('synthetic-proof'));
  await writeFile(path.join(profile, 'env/synthetic.env'), `# preserved\nA=enc:${safeStorage.encryptString('synthetic-env').toString('base64')}\nPLAIN="enc:literal"\n`);
  await writeFile(path.join(profile, 'agent-provider.json'), JSON.stringify({ secret: safeStorage.encryptString('synthetic-old').toString('base64'), secrets: { 'https://synthetic.invalid': safeStorage.encryptString('synthetic-provider').toString('base64') } }));
}
it('discovers all four families without decrypting or touching their encrypted source bytes', async () => {
  await fixture();
  const source = await readFile(path.join(profile, 'env/synthetic.env'));
  const records = await discoverSecrets(profile);
  expect(records).toHaveLength(5); expect(records.map(x => x.locator)).toContain('env/synthetic.env:A');
  expect(await readFile(path.join(profile, 'env/synthetic.env'))).toEqual(source);
});
it('writes versioned Keychain items with both requirements, zeros buffers and resumes from secret-free receipts', async () => {
  await fixture(); const buffers: Uint8Array[] = [], seen: string[] = [];
  const put = vi.fn<LegacyKeychain['putLegacyItem']>(async input => {
    expect(input.designatedRequirements).toEqual(requirements);
    buffers.push(input.value); seen.push(Buffer.from(input.value).toString());
    return { persistentRef: 'synthetic-reference', designatedRequirements: input.designatedRequirements };
  });
  const handoff = new SecretHandoff({ profile, safeStorage, keychain: { putLegacyItem: put }, designatedRequirements: requirements });
  await handoff.run(); await handoff.run(); expect(put).toHaveBeenCalledTimes(5);
  expect(buffers.every(b => b.every(x => x === 0))).toBe(true);
  const journal = await readFile(path.join(profile, 'Media Store/secret-handoff.json'), 'utf8');
  for (const secret of seen) expect(journal).not.toContain(secret);
  await writeFile(path.join(profile, 'cloud/session.bin'), safeStorage.encryptString('synthetic-changed'));
  await handoff.run(); expect(put).toHaveBeenCalledTimes(6);
});
it('fails closed on unavailable signing, locked safeStorage and a broadened ACL', async () => {
  await fixture();
  await expect(new SecretHandoff({ profile, safeStorage }).run()).rejects.toThrow('signed');
  const keychain: LegacyKeychain = { putLegacyItem: async () => ({ persistentRef: 'synthetic', designatedRequirements: [requirements[0]] }) };
  await expect(new SecretHandoff({ profile, safeStorage: { ...safeStorage, isEncryptionAvailable: () => false }, keychain, designatedRequirements: requirements }).run()).rejects.toThrow('Unlock');
  await expect(new SecretHandoff({ profile, safeStorage, keychain, designatedRequirements: requirements }).run()).rejects.toThrow('not verified');
  expect(await readdir(profile)).not.toContain('Media Store');
});
it('rejects concurrent secret changes and sanitizes backend errors', async () => {
  await fixture(); let changed = false;
  const keychain: LegacyKeychain = { putLegacyItem: async input => {
    if (!changed) { changed = true; await writeFile(path.join(profile, 'cloud/session.bin'), safeStorage.encryptString('synthetic-delta')); }
    return { persistentRef: 'synthetic-ref', designatedRequirements: input.designatedRequirements };
  } };
  await expect(new SecretHandoff({ profile, safeStorage, keychain, designatedRequirements: requirements }).run()).rejects.toThrow('changed');
  keychain.putLegacyItem = async () => { throw new Error('never-expose-synthetic-secret'); };
  await expect(new SecretHandoff({ profile, safeStorage, keychain, designatedRequirements: requirements }).run()).rejects.toThrow('not verified');
});
it('does not invent signed identities on Linux', async () => { expect(await signedKeychain(profile)).toEqual({}); });
