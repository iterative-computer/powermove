/*
 * A self-signed certificate for `powermove serve`. Browsers only expose
 * WebCodecs, OPFS, the clipboard and other APIs the editor needs on a secure
 * origin, and a LAN or tailnet address over plain http is not one. The
 * certificate is minted with Node's native crypto, lists every address the
 * host is reachable on, and is reused until that list changes.
 *
 * Browsers still warn once per origin ("proceed anyway"); `tailscale serve`
 * in front of `--http` mode gives a trusted certificate instead.
 */
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { networkInterfaces } from 'node:os';
import path from 'node:path';

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
export async function ensureCertificate(dir: string, addresses = hostAddresses()): Promise<TlsMaterial> {
  await mkdir(dir, { recursive: true });
  const keyPath = path.join(dir, 'serve-key.pem');
  const certPath = path.join(dir, 'serve-cert.pem');
  const sansPath = path.join(dir, 'serve-cert.sans');
  const sans = subjectAltNames(addresses);
  try {
    const [key, cert, previous] = await Promise.all([readFile(keyPath, 'utf8'), readFile(certPath, 'utf8'), readFile(sansPath, 'utf8')]);
    if (previous.trim() === sans) return { key, cert };
  } catch { /* first run or incomplete */ }
  // Windows does not include an openssl command. Generate the same PEM
  // material on every host without installing a separate command-line tool.
  const { generate } = await import('selfsigned');
  const certificate = await generate([{ name: 'commonName', value: 'Powermove serve' }], {
    keyType: 'rsa', keySize: 2048, algorithm: 'sha256',
    notAfterDate: new Date(Date.now() + 3650 * 24 * 60 * 60 * 1000),
    extensions: [
      { name: 'basicConstraints', cA: false },
      { name: 'keyUsage', digitalSignature: true, keyEncipherment: true },
      { name: 'extKeyUsage', serverAuth: true },
      { name: 'subjectAltName', altNames: [{ type: 2, value: 'localhost' }, ...addresses.map(ip => ({ type: 7 as const, ip }))] }
    ]
  });
  await writeFile(keyPath, certificate.private, { mode: 0o600 });
  await writeFile(certPath, certificate.cert, { mode: 0o600 });
  await writeFile(sansPath, sans, { mode: 0o600 });
  return { key: await readFile(keyPath, 'utf8'), cert: await readFile(certPath, 'utf8') };
}
