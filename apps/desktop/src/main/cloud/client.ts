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

export type CloudApi = ReturnType<typeof hc<AppType>>;

export interface CloudClientOptions {
  /** `https://cloud.trypowermove.com` or a self-hosted registry. */
  origin: string;
  /** The session token for `origin`, or null when signed out. */
  getToken(): string | null | Promise<string | null>;
  appVersion: string;
  /** Test seam; defaults to the global fetch. */
  fetch?: typeof fetch;
}

export interface CloudClient {
  readonly origin: string;
  readonly api: CloudApi;
  /** Run one call and parse its body. `schema` null means "no body expected". */
  request<T>(schema: z.ZodType<T>, fn: (api: CloudApi) => Promise<Response>): Promise<T>;
  request(schema: null, fn: (api: CloudApi) => Promise<Response>): Promise<void>;
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
  const clientHeader = `desktop/${options.appVersion}`;
  return async (input, init) => {
    const url = requestUrl(input);
    const headers = new Headers(init?.headers ?? (input instanceof Request ? input.headers : undefined));
    headers.delete('Authorization');
    headers.set(CLIENT_HEADER, clientHeader);
    if (url.origin === origin) {
      const token = await options.getToken();
      if (token) headers.set('Authorization', `Bearer ${token}`);
    }
    return fetchImpl(url, { ...init, headers, redirect: 'error' });
  };
}

export function createCloudClient(options: CloudClientOptions): CloudClient {
  const origin = normalizeOrigin(options.origin);
  const bound = createBoundFetch(options);

  const api: CloudApi = hc<AppType>(origin, { fetch: bound });

  function request<T>(schema: z.ZodType<T>, fn: (api: CloudApi) => Promise<Response>): Promise<T>;
  function request(schema: null, fn: (api: CloudApi) => Promise<Response>): Promise<void>;
  async function request<T>(schema: z.ZodType<T> | null, fn: (api: CloudApi) => Promise<Response>): Promise<T | void> {
    let response: Response;
    try {
      response = await fn(api);
    } catch (error) {
      if (error instanceof ApiError) throw error;
      throw new ApiError({ error: 'internal', detail: 'Unable to reach Powermove Cloud. Check your connection and try again.' });
    }
    const body = await readJson(response);
    if (!response.ok) {
      const parsed = body.ok ? ApiErrorBody.safeParse(body.value) : null;
      if (parsed?.success) throw new ApiError(parsed.data);
      throw new ApiError({ error: 'internal', detail: `Powermove Cloud is unavailable right now (${response.status}). Try again in a moment.` });
    }
    if (schema === null) return;
    if (!body.ok) throw new ApiError({ error: 'internal', detail: `Powermove Cloud is unavailable right now (${response.status}). Try again in a moment.` });
    const parsed = schema.safeParse(body.value);
    if (!parsed.success) throw new ApiError({ error: 'internal', detail: 'This version of Powermove can’t read the response from Powermove Cloud. Update Powermove, then try again.' });
    return parsed.data;
  }

  return { origin, api, request };
}
