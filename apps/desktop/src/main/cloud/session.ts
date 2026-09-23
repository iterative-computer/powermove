/*
 * The Powermove Cloud session on this Mac (store plan §2.3 step 6, P0 amendments).
 *
 *   <userData>/cloud/session.bin   safeStorage-sealed JSON { origin, token, expiresAt }
 *   <userData>/cloud/session.json  non-secret cache { origin, me, fetchedAt, provider? }
 *
 * The token is bound to the origin it was issued by: `currentToken()` answers
 * only while that origin is still the configured registry, and the client
 * sends it only to that origin. `me` is cached so ownership and the account
 * row survive being offline; it is refreshed from `GET /v1/me` when online.
 *
 * safeStorage is injected: it only works after `app.whenReady()`, and tests
 * pass a fake. Tokens are never logged.
 */
import { mkdir, readFile, rename, rm, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { ApiError, Auth, Me, MeDto, type SessionDto } from '@powermove/registry/wire';
import { z } from 'zod';

import type { CloudProvider } from '../../shared/cloud-ipc';
import type { SafeStorageLike } from '../env/store';
import { createCloudClient, normalizeOrigin, type CloudClient } from './client';

const Secret = z.object({ origin: z.string(), token: z.string().min(1), expiresAt: z.string() });
type Secret = z.infer<typeof Secret>;
const Cache = z.object({
  origin: z.string(),
  me: MeDto.nullable(),
  fetchedAt: z.string(),
  provider: z.enum(['google', 'github', 'email']).optional()
});
type Cache = z.infer<typeof Cache>;

export interface CloudSessionOptions {
  /** `<userData>/cloud`. */
  dir: string;
  safeStorage(): SafeStorageLike;
  /** The configured registry origin (Settings › Advanced › Registry URL). */
  origin(): string;
  appVersion: string;
  /** Test seams. */
  fetch?: typeof fetch;
  now?(): number;
}

export interface CloudSession {
  readonly dir: string;
  load(): Promise<void>;
  /** Store a new bearer session for the current origin. */
  save(session: SessionDto, provider?: CloudProvider): Promise<void>;
  /** Forget the session and the cached account. */
  clear(): Promise<void>;
  currentToken(): string | null;
  /** The cached account for the current origin, or null. */
  me(): MeDto | null;
  provider(): CloudProvider | undefined;
  /** A client for the current origin that sends this session's token. */
  client(): CloudClient;
  /** GET /v1/me → cache. A 401 means the session is gone: it is cleared and null returned. */
  refreshMe(): Promise<MeDto | null>;
  /** POST /v1/auth/sign-out, best effort, then clear. */
  signOut(): Promise<void>;
}

export function createCloudSession(options: CloudSessionOptions): CloudSession {
  const now = options.now ?? Date.now;
  const binFile = path.join(options.dir, 'session.bin');
  const jsonFile = path.join(options.dir, 'session.json');
  let secret: Secret | null = null;
  let cache: Cache | null = null;
  let cachedClient: CloudClient | null = null;

  const live = (): Secret | null => {
    if (!secret || secret.origin !== options.origin()) return null;
    const expires = Date.parse(secret.expiresAt);
    return Number.isFinite(expires) && expires > now() ? secret : null;
  };

  async function writeAtomic(file: string, data: string | Buffer): Promise<void> {
    await mkdir(options.dir, { recursive: true, mode: 0o700 });
    await writeFile(`${file}.tmp`, data, { mode: 0o600 });
    await rename(`${file}.tmp`, file);
  }

  async function writeCache(next: Cache): Promise<void> {
    cache = next;
    await writeAtomic(jsonFile, JSON.stringify(next));
  }

  const session: CloudSession = {
    dir: options.dir,

    async load() {
      secret = null;
      cache = null;
      try {
        const storage = options.safeStorage();
        if (storage.isEncryptionAvailable()) {
          const parsed = Secret.safeParse(JSON.parse(storage.decryptString(await readFile(binFile))));
          if (parsed.success) secret = parsed.data;
        }
      } catch {
        // Missing, unreadable or sealed by another Mac: signed out.
      }
      try {
        const parsed = Cache.safeParse(JSON.parse(await readFile(jsonFile, 'utf8')));
        if (parsed.success && secret && parsed.data.origin === secret.origin) cache = parsed.data;
      } catch {
        // No cache yet.
      }
      if (secret && !live() && secret.origin === options.origin()) {
        // Expired: nothing to keep.
        await session.clear();
      }
    },

    async save(dto, provider) {
      const storage = options.safeStorage();
      if (!storage.isEncryptionAvailable()) {
        throw new ApiError({ error: 'internal', detail: 'Secure storage is unavailable, so Powermove can’t keep you signed in. Unlock your Mac and try again.' });
      }
      const next: Secret = { origin: options.origin(), token: dto.token, expiresAt: dto.expiresAt };
      await writeAtomic(binFile, storage.encryptString(JSON.stringify(next)));
      secret = next;
      await writeCache({ origin: next.origin, me: null, fetchedAt: new Date(now()).toISOString(), ...(provider ? { provider } : {}) });
    },

    async clear() {
      secret = null;
      cache = null;
      await Promise.all([rm(binFile, { force: true }), rm(jsonFile, { force: true })]);
    },

    currentToken: () => live()?.token ?? null,

    me: () => (live() && cache?.origin === options.origin() ? cache.me : null),

    provider: () => (live() ? cache?.provider : undefined),

    client() {
      const origin = options.origin();
      if (!cachedClient || cachedClient.origin !== normalizeOrigin(origin)) {
        cachedClient = createCloudClient({
          origin,
          appVersion: options.appVersion,
          getToken: () => session.currentToken(),
          ...(options.fetch ? { fetch: options.fetch } : {})
        });
      }
      return cachedClient;
    },

    async refreshMe() {
      const current = live();
      if (!current) return null;
      try {
        const me = await session.client().request(Me.Get.Res, (api) => api.v1.me.$get());
        await writeCache({ origin: current.origin, me, fetchedAt: new Date(now()).toISOString(), ...(cache?.provider ? { provider: cache.provider } : {}) });
        return me;
      } catch (error) {
        if (error instanceof ApiError && error.body.error === 'unauthorized') {
          await session.clear();
          return null;
        }
        throw error;
      }
    },

    async signOut() {
      if (live()) {
        try {
          await session.client().request(Auth.SignOut.Res, (api) => api.v1.auth['sign-out'].$post());
        } catch {
          // Best effort: the local session goes either way.
        }
      }
      await session.clear();
    }
  };
  return session;
}
