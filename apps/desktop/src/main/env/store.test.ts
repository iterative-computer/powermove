import * as fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import { afterEach, describe, expect, it } from 'vitest';

import { createEnvStore, envFileName, SecretStorageUnavailableError, type SafeStorageLike } from './store';

const roots: string[] = [];

async function tempDir(): Promise<string> {
  const dir = await fs.mkdtemp(path.join(os.tmpdir(), 'powermove-env-'));
  roots.push(dir);
  return path.join(dir, 'env');
}

afterEach(async () => {
  await Promise.all(roots.splice(0).map((dir) => fs.rm(dir, { recursive: true, force: true })));
});

function fakeSafeStorage(available = true): SafeStorageLike & { available: boolean } {
  const fake = {
    available,
    isEncryptionAvailable: () => fake.available,
    encryptString: (text: string) => Buffer.from(`sealed:${text}`, 'utf8'),
    decryptString: (buffer: Buffer) => {
      const text = buffer.toString('utf8');
      if (!text.startsWith('sealed:')) throw new Error('cannot decrypt');
      return text.slice('sealed:'.length);
    }
  };
  return fake;
}

describe('env store', () => {
  it('round-trips plain values, including ones that need quoting', async () => {
    const dir = await tempDir();
    const store = createEnvStore({ dir, safeStorage: fakeSafeStorage() });
    await store.writeVar('local:abc', 'REGION', 'eu-west-1', false);
    await store.writeVar('local:abc', 'GREETING', ' hello # world\nsecond line', false);
    await store.writeVar('local:abc', 'LOOKS_SEALED', 'enc:not-really', false);
    await store.writeVar('local:abc', 'REGION', 'us-east-1', false);

    const env = await store.readEnv('local:abc');
    expect(env.get('REGION')).toEqual({ value: 'us-east-1', secret: false });
    expect(env.get('GREETING')).toEqual({ value: ' hello # world\nsecond line', secret: false });
    expect(env.get('LOOKS_SEALED')).toEqual({ value: 'enc:not-really', secret: false });

    const text = await fs.readFile(store.fileFor('local:abc'), 'utf8');
    expect(text).toContain('REGION=us-east-1\n');
    expect(text).toContain('GREETING=" hello # world\\nsecond line"');
    expect(text.match(/^REGION=/gm)).toHaveLength(1);
  });

  it('keeps comments, reads inline comments and names the file from the env key', async () => {
    const dir = await tempDir();
    const store = createEnvStore({ dir, safeStorage: fakeSafeStorage() });
    await fs.mkdir(dir, { recursive: true });
    await fs.writeFile(path.join(dir, 'local_abc.env'), '# mine\nMODE=fast # trailing\n\nexport NOPE=1\n');
    expect((await store.readEnv('local:abc')).get('MODE')?.value).toBe('fast');
    await store.writeVar('local:abc', 'TOKEN', 'x', false);
    expect(await fs.readFile(path.join(dir, 'local_abc.env'), 'utf8')).toBe('# mine\nMODE=fast # trailing\n\nexport NOPE=1\nTOKEN=x\n');
    expect(envFileName('repo/with:colon')).toBe('repo_with_colon.env');
    expect(() => envFileName('..')).toThrow();
    expect(() => envFileName('../x')).toThrow();
  });

  it('seals secrets and never writes them in plaintext', async () => {
    const dir = await tempDir();
    const safe = fakeSafeStorage();
    const store = createEnvStore({ dir, safeStorage: () => safe });
    await store.writeVar('local:k', 'API_KEY', 'sk-live-value', true);
    const text = await fs.readFile(store.fileFor('local:k'), 'utf8');
    expect(text).not.toContain('sk-live-value');
    expect(text).toMatch(/^API_KEY=enc:[A-Za-z0-9+/=]+$/m);
    expect((await store.readEnv('local:k')).get('API_KEY')).toEqual({ value: 'sk-live-value', secret: true });
  });

  it('re-seals a plaintext secret on load', async () => {
    const dir = await tempDir();
    const store = createEnvStore({ dir, safeStorage: fakeSafeStorage() });
    await store.writeVar('local:k', 'API_KEY', 'typed-by-hand', false);
    const env = await store.readEnv('local:k', new Set(['API_KEY']));
    expect(env.get('API_KEY')).toEqual({ value: 'typed-by-hand', secret: true });
    const text = await fs.readFile(store.fileFor('local:k'), 'utf8');
    expect(text).not.toContain('typed-by-hand');
    expect(text).toMatch(/^API_KEY=enc:/m);
  });

  it('marks a sealed value this Mac cannot open as undecryptable', async () => {
    const dir = await tempDir();
    const store = createEnvStore({ dir, safeStorage: fakeSafeStorage() });
    await fs.mkdir(dir, { recursive: true });
    await fs.writeFile(store.fileFor('local:k'), `API_KEY=enc:${Buffer.from('from another mac').toString('base64')}\n`);
    expect((await store.readEnv('local:k')).get('API_KEY')).toEqual({ value: null, secret: true });
  });

  it('refuses to save a secret when secure storage is unavailable', async () => {
    const dir = await tempDir();
    const store = createEnvStore({ dir, safeStorage: fakeSafeStorage(false) });
    await expect(store.writeVar('local:k', 'API_KEY', 'value', true)).rejects.toBeInstanceOf(SecretStorageUnavailableError);
    expect(await store.exists('local:k')).toBe(false);
  });

  it('writes the directory 0700 and files 0600', async () => {
    const dir = await tempDir();
    const store = createEnvStore({ dir, safeStorage: fakeSafeStorage() });
    await store.writeVar('local:k', 'REGION', 'eu', false);
    expect((await fs.stat(dir)).mode & 0o777).toBe(0o700);
    expect((await fs.stat(store.fileFor('local:k'))).mode & 0o777).toBe(0o600);
    expect((await fs.readdir(dir)).filter((name) => name.endsWith('.tmp'))).toEqual([]);
  });

  it('deletes one value and the whole file', async () => {
    const dir = await tempDir();
    const store = createEnvStore({ dir, safeStorage: fakeSafeStorage() });
    await store.writeVar('local:k', 'A_KEY', '1', false);
    await store.writeVar('local:k', 'B_KEY', '2', false);
    await store.deleteVar('local:k', 'A_KEY');
    expect([...(await store.readEnv('local:k')).keys()]).toEqual(['B_KEY']);
    await store.deleteEnv('local:k');
    expect(await store.exists('local:k')).toBe(false);
    expect((await store.readEnv('local:k')).size).toBe(0);
  });
});
