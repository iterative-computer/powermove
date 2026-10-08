import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import { X509Certificate } from 'node:crypto';
import { createServer, get } from 'node:https';

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

  it('serves a verifiable TLS connection without an openssl executable', async () => {
    dir = await mkdtemp(path.join(tmpdir(), 'pm-tls-'));
    const material = await ensureCertificate(dir, ['127.0.0.1']);
    const cert = new X509Certificate(material.cert);
    expect(cert.checkHost('localhost')).toBe('localhost');
    expect(cert.checkIP('127.0.0.1')).toBe('127.0.0.1');
    expect(cert.ca).toBe(false);
    const server = createServer(material, (_request, response) => response.end('secure'));
    await new Promise<void>(resolve => server.listen(0, '127.0.0.1', resolve));
    try {
      const address = server.address();
      if (!address || typeof address === 'string') throw new Error('No TLS listening address');
      const body = await new Promise<string>((resolve, reject) => {
        get({ host: '127.0.0.1', port: address.port, ca: material.cert, servername: 'localhost' }, response => {
          let data = ''; response.on('data', chunk => { data += chunk; }); response.on('end', () => resolve(data));
        }).on('error', reject);
      });
      expect(body).toBe('secure');
    } finally { await new Promise<void>(resolve => server.close(() => resolve())); }
  });
});
