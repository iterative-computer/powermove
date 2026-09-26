import { app, BrowserWindow, safeStorage, shell } from 'electron';
import { createHash, randomUUID } from 'node:crypto';
import { mkdir, readFile, rename, writeFile, rm } from 'node:fs/promises';
import path from 'node:path';
import { ApiError } from '@powermove/registry/wire';

type Saved = { origin: string; scope: string; clearance: string; expires: number };
const pending = new Map<string, Promise<string>>();

/** The browser carries only a signed challenge, never the app's session or an email address. */
export function verifyHuman(origin: string, ticket: string, scope: string): Promise<string> {
  const key = `${origin}\n${scope}`;
  const existing = pending.get(key);
  if (existing) return existing;
  const work = run(origin, ticket, scope).finally(() => pending.delete(key));
  pending.set(key, work);
  return work;
}

export async function forgetHumanVerification(origin: string, scope: string): Promise<void> {
  const name = `${createHash('sha256').update(`${origin}\n${scope}`).digest('hex')}.bin`;
  await rm(path.join(app.getPath('userData'), 'cloud', 'human', name), { force: true }).catch(() => undefined);
}

async function run(origin: string, ticket: string, scope: string): Promise<string> {
  const dir = path.join(app.getPath('userData'), 'cloud', 'human');
  const file = path.join(dir, `${createHash('sha256').update(`${origin}\n${scope}`).digest('hex')}.bin`);
  try {
    if (safeStorage.isEncryptionAvailable()) {
      const cached = JSON.parse(safeStorage.decryptString(await readFile(file))) as Saved;
      if (cached.origin === origin && cached.scope === scope && cached.expires > Date.now() + 60_000 && typeof cached.clearance === 'string') return cached.clearance;
    }
  } catch { /* New device, expired or unavailable secure storage: verify again. */ }
  await rm(file, { force: true }).catch(() => undefined);
  const page = new URL('/v1/human', origin);
  page.searchParams.set('ticket', ticket);
  await shell.openExternal(page.href);
  const deadline = Date.now() + 3 * 60_000;
  while (Date.now() < deadline) {
    await new Promise(resolve => setTimeout(resolve, 1500));
    const response = await fetch(new URL('/v1/human/result', origin), {
      method: 'POST', redirect: 'error', headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ ticket }), signal: AbortSignal.timeout(10_000)
    }).catch(() => null);
    if (!response || response.status === 204) continue;
    if (!response.ok) break;
    const result = await response.json() as { clearance?: unknown };
    if (typeof result.clearance !== 'string' || result.clearance.length > 2048) break;
    try {
      const proof = JSON.parse(Buffer.from(result.clearance.split('.')[0]!, 'base64url').toString()) as { expires: number };
      if (safeStorage.isEncryptionAvailable() && Number.isFinite(proof.expires)) {
        await mkdir(dir, { recursive: true, mode: 0o700 });
        const temp = `${file}.${randomUUID()}.tmp`;
        try {
          await writeFile(temp, safeStorage.encryptString(JSON.stringify({ origin, scope, clearance: result.clearance, expires: proof.expires } satisfies Saved)), { mode: 0o600 });
          await rename(temp, file);
        } finally { await rm(temp, { force: true }).catch(() => undefined); }
      }
    } catch { /* Verification can succeed even when remembering it is unavailable. */ }
    const window = BrowserWindow.getAllWindows().find(candidate => !candidate.isDestroyed());
    window?.show(); window?.focus();
    return result.clearance;
  }
  throw new ApiError({ error: 'bad_request', detail: 'Verification wasn’t completed. Please try again.' });
}
