/*
 * A self-signed certificate for `powermove serve`. Browsers only expose
 * WebCodecs, OPFS, the clipboard and other APIs the editor needs on a secure
 * origin, and a LAN or tailnet address over plain http is not one. The
 * certificate is minted with the machine's openssl, lists every address the
 * host is reachable on, and is reused until that list changes.
 *
 * Browsers still warn once per origin ("proceed anyway"); `tailscale serve`
 * in front of `--http` mode gives a trusted certificate instead.
 */
import { execFile } from 'node:child_process';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { networkInterfaces } from 'node:os';
import path from 'node:path';
import { promisify } from 'node:util';

const run = promisify(execFile);

export interface TlsMaterial { key: string; cert: string }

export function hostAddresses(): string[] {
  const addresses = new Set<string>(['127.0.0.1']);
  for (const entries of Object.values(networkInterfaces())) {
    for (const entry of entries ?? []) if (!entry.internal && entry.family === 'IPv4') addresses.add(entry.address);
  }
  return [...addresses].sort();
}

export function subjectAltNames(addresses: string[]): string {
  return ['DNS:localhost', ...addresses.map((address) => `IP:${address}`)].join(',');
}

/** Reads the profile's certificate, minting a new one when addresses changed. */
export async function ensureCertificate(dir: string, addresses = hostAddresses(), openssl = 'openssl'): Promise<TlsMaterial> {
  await mkdir(dir, { recursive: true });
  const keyPath = path.join(dir, 'serve-key.pem');
  const certPath = path.join(dir, 'serve-cert.pem');
  const sansPath = path.join(dir, 'serve-cert.sans');
  const sans = subjectAltNames(addresses);
  try {
    const [key, cert, previous] = await Promise.all([readFile(keyPath, 'utf8'), readFile(certPath, 'utf8'), readFile(sansPath, 'utf8')]);
    if (previous.trim() === sans) return { key, cert };
  } catch { /* first run or incomplete */ }
  try {
    await run(openssl, [
      // RSA: LibreSSL's EC keys carry explicit curve parameters, which Node's OpenSSL rejects.
      'req', '-x509', '-newkey', 'rsa:2048', '-nodes',
      '-keyout', keyPath, '-out', certPath, '-days', '3650',
      '-subj', '/CN=Powermove serve', '-addext', `subjectAltName=${sans}`
    ]);
  } catch (error) {
    throw new Error(`Could not create a certificate with openssl (${error instanceof Error ? error.message : String(error)}). Install openssl, or run with --http behind a proxy that terminates TLS.`);
  }
  await writeFile(sansPath, sans, { mode: 0o600 });
  return { key: await readFile(keyPath, 'utf8'), cert: await readFile(certPath, 'utf8') };
}
