import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';

import { ensureCertificate, subjectAltNames } from './tls';

let dir: string;
afterEach(async () => { if (dir) await rm(dir, { recursive: true, force: true }); });

describe('ensureCertificate', () => {
  it('builds SANs for localhost and every address', () => {
    expect(subjectAltNames(['127.0.0.1', '100.64.0.2'])).toBe('DNS:localhost,IP:127.0.0.1,IP:100.64.0.2');
  });

  it('mints once and reuses until the addresses change', async () => {
    dir = await mkdtemp(path.join(tmpdir(), 'pm-tls-'));
    const first = await ensureCertificate(dir, ['127.0.0.1']);
    expect(first.cert).toContain('BEGIN CERTIFICATE');
    expect(first.key).toContain('PRIVATE KEY');
    const again = await ensureCertificate(dir, ['127.0.0.1']);
    expect(again.cert).toBe(first.cert);
    const changed = await ensureCertificate(dir, ['127.0.0.1', '192.168.1.9']);
    expect(changed.cert).not.toBe(first.cert);
    expect((await readFile(path.join(dir, 'serve-cert.sans'), 'utf8'))).toContain('IP:192.168.1.9');
  });

  it('explains a missing openssl', async () => {
    dir = await mkdtemp(path.join(tmpdir(), 'pm-tls-'));
    await expect(ensureCertificate(dir, ['127.0.0.1'], '/nonexistent/openssl')).rejects.toThrow(/openssl/);
  });
});
