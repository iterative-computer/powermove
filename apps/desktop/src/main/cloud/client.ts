/*
 * The typed Powermove Cloud client (store plan §2.7).
 *
 * `hc<AppType>` over the cloud's emitted declarations gives route and method
 * names checked at compile time; `AppType` comes from
 * `apps/cloud/dist/types/index.d.ts` through the `@powermove/cloud-types` path
 * in `tsconfig.node.json` (type-only; no cloud runtime code is bundled).
 *
 * Every call goes through `request(schema, fn)`:
 *   - every request carries `X-Powermove-Client: desktop/<version>` (§2.6b);
 *   - 2xx bodies are parsed with the wire response schema;
 *   - non-2xx bodies are parsed with `ApiErrorBody` and thrown as `ApiError`;
 *     anything unparsable becomes `ApiError({ error: 'internal', detail })`.
 *
 * The bearer is bound to the configured origin: the fetch wrapper strips any
 * Authorization header and adds the session token only when the request URL's
 * origin equals it, and redirects are refused so a token never follows one.
 */
import { hc } from 'hono/client';
import type { AppType } from '@powermove/cloud-types';
import { ApiError, ApiErrorBody, CLIENT_HEADER } from '@powermove/registry/wire';
import type { z } from 'zod';

import { CLOUD_UNREACHABLE } from '../../shared/cloud-ipc';

export type CloudApi = ReturnType<typeof hc<AppType>>;

export interface CloudClientOptions {
  /** `https://cloud.trypowermove.com` or a self-hosted registry. */
  origin: string;
  /** The session token for `origin`, or null when signed out. */
  getToken(): string | null | Promise<string | null>;
  appVersion: string;
  /** Test seam; defaults to the global fetch. */
  fetch?: typeof fetch;
  verifyHuman?(origin: string, ticket: string, scope: string): Promise<string>;
}

export interface CloudClient {
  readonly origin: string;
  readonly api: CloudApi;
  /** Run one call and parse its body. `schema` null means "no body expected". */
  request<T>(schema: z.ZodType<T>, fn: (api: CloudApi) => Promise<Response>): Promise<T>;
  request(schema: null, fn: (api: CloudApi) => Promise<Response>): Promise<void>;
  /**
   * Run one call whose success body is not JSON (a tar, a source file) and
   * return the response unread. Failures are thrown as for `request`. `fetch`
   * is the origin-bound fetch, for routes the typed client cannot address
   * (a wildcard path).
   */
  raw(fn: (api: CloudApi, fetch: typeof globalThis.fetch) => Promise<Response>): Promise<Response>;
}

/** The scheme, host and port of a registry URL; throws on anything that is not http(s). */
export function normalizeOrigin(value: string): string {
  const url = new URL(value);
  if (url.protocol !== 'https:' && url.protocol !== 'http:') throw new Error('The registry URL must start with https://');
  if (url.username || url.password) throw new Error('The registry URL cannot contain a user name or password');
  return url.origin;
}

function requestUrl(input: RequestInfo | URL): URL {
  if (input instanceof URL) return input;
  if (typeof input === 'string') return new URL(input);
  return new URL(input.url);
}

async function readJson(response: Response): Promise<{ ok: true; value: unknown } | { ok: false }> {
  try {
    const text = await response.text();
    return { ok: true, value: text.length ? JSON.parse(text) : undefined };
  } catch {
    return { ok: false };
  }
}

/**
 * The fetch every cloud request goes through: adds the client header, strips
 * any Authorization it was handed, and adds the bearer only for `origin`.
 * Redirects are refused so the token never follows one.
 */
export function createBoundFetch(options: CloudClientOptions): typeof fetch {
  const origin = normalizeOrigin(options.origin);
  const fetchImpl = options.fetch ?? fetch;
  const clearances = new Map<string, { value: string; expires: number }>();
  const clientHeader = `desktop/${options.appVersion}`;
  return async (input, init) => {
    const url = requestUrl(input);
    const headers = new Headers(init?.headers ?? (input instanceof Request ? input.headers : undefined));
    headers.delete('Authorization');
    headers.delete('X-Powermove-Human');
    headers.set(CLIENT_HEADER, clientHeader);
    if (url.origin === origin) {
      const token = await options.getToken();
      if (token) headers.set('Authorization', `Bearer ${token}`);
    }
    const response = await fetchImpl(url, { ...init, headers, redirect: 'error' });
    if (url.origin !== origin || response.status !== 403) return response;
    const challenge = ApiErrorBody.safeParse(await response.clone().json().catch(() => null));
    if (!challenge.success || challenge.data.error !== 'human_verification_required') return response;
    const { ticket, scope } = challenge.data;
    const cached = clearances.get(scope);
    const verify = options.verifyHuman ?? (async (target: string, proof: string, key: string) => (await import('./human-verification')).verifyHuman(target, proof, key));
    const clearance = cached && cached.expires > Date.now() ? cached.value : await verify(origin, ticket, scope);
    if (clearances.size >= 16 && !clearances.has(scope)) clearances.clear();
    clearances.set(scope, { value: clearance, expires: cached?.value === clearance ? cached.expires : Date.now() + 9 * 60_000 });
    // Protected routes reject before their side effects. Retry only once, never loop challenges.
    headers.set('X-Powermove-Human', clearance);
    const retried = await fetchImpl(url, { ...init, headers, redirect: 'error' });
    if (retried.status === 403) {
      clearances.delete(scope);
      if (!options.verifyHuman) await (await import('./human-verification')).forgetHumanVerification(origin, scope);
    }
    return retried;
  };
}

export function createCloudClient(options: CloudClientOptions): CloudClient {
  const origin = normalizeOrigin(options.origin);
  const bound = createBoundFetch(options);

  const api: CloudApi = hc<AppType>(origin, { fetch: bound });

  async function send(fn: (api: CloudApi, fetch: typeof globalThis.fetch) => Promise<Response>): Promise<Response> {
    try {
      return await fn(api, bound);
    } catch (error) {
      if (error instanceof ApiError) throw error;
      throw new ApiError({ error: 'internal', detail: CLOUD_UNREACHABLE });
    }
  }

  async function failure(response: Response, body: { ok: true; value: unknown } | { ok: false }): Promise<never> {
    const parsed = body.ok ? ApiErrorBody.safeParse(body.value) : null;
    if (parsed?.success) throw new ApiError(parsed.data);
    throw new ApiError({ error: 'internal', detail: `Powermove Cloud is unavailable right now (${response.status}). Try again in a moment.` });
  }

  function request<T>(schema: z.ZodType<T>, fn: (api: CloudApi) => Promise<Response>): Promise<T>;
  function request(schema: null, fn: (api: CloudApi) => Promise<Response>): Promise<void>;
  async function request<T>(schema: z.ZodType<T> | null, fn: (api: CloudApi) => Promise<Response>): Promise<T | void> {
    const response = await send(fn);
    const body = await readJson(response);
    if (!response.ok) return failure(response, body);
    if (schema === null) return;
    if (!body.ok) throw new ApiError({ error: 'internal', detail: `Powermove Cloud is unavailable right now (${response.status}). Try again in a moment.` });
    const parsed = schema.safeParse(body.value);
    if (!parsed.success) throw new ApiError({ error: 'internal', detail: 'This version of Powermove can’t read the response from Powermove Cloud. Update Powermove, then try again.' });
    return parsed.data;
  }

  async function raw(fn: (api: CloudApi, fetch: typeof globalThis.fetch) => Promise<Response>): Promise<Response> {
    const response = await send(fn);
    if (!response.ok) return failure(response, await readJson(response));
    return response;
  }

  return { origin, api, request, raw };
}
