import { describe, expect, it, vi } from 'vitest';
import { ApiError, Auth, Me } from '@powermove/registry/wire';

import { createBoundFetch, createCloudClient } from './client';

const ORIGIN = 'https://cloud.example.test';
const ME = {
  user: { id: '3f1c9b1e-8f55-4d8f-9d0a-6f1d1c1b2a3e', name: 'Jude', email: 'jude@example.test', image: null },
  publisher: null,
  settings: { rememberInstalls: true }
};

type Call = { url: URL; init: RequestInit; headers: Headers };

function mockFetch(respond: (url: URL) => Response) {
  const calls: Call[] = [];
  const fetch = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = input instanceof URL ? input : new URL(String(input));
    calls.push({ url, init: init ?? {}, headers: new Headers(init?.headers) });
    return respond(url);
  });
  return { calls, fetch: fetch as unknown as typeof globalThis.fetch };
}

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });

describe('cloud client', () => {
  it('sends the client header and the bearer, and parses a 2xx body', async () => {
    const { calls, fetch } = mockFetch(() => json(ME));
    const client = createCloudClient({ origin: ORIGIN, getToken: () => 'tok-1', appVersion: '1.2.3', fetch });
    const me = await client.request(Me.Get.Res, (api) => api.v1.me.$get());
    expect(me).toEqual(ME);
    expect(calls).toHaveLength(1);
    expect(calls[0]!.url.toString()).toBe(`${ORIGIN}/v1/me`);
    expect(calls[0]!.headers.get('X-Powermove-Client')).toBe('desktop/1.2.3');
    expect(calls[0]!.headers.get('Authorization')).toBe('Bearer tok-1');
    expect(calls[0]!.init.redirect).toBe('error');
  });

  it('sends the client header even when signed out', async () => {
    const { calls, fetch } = mockFetch(() => json({ ok: true }));
    const client = createCloudClient({ origin: ORIGIN, getToken: () => null, appVersion: '0.9.0', fetch });
    await client.request(Auth.EmailSend.Res, (api) => api.v1.auth.email.send.$post({ json: { email: 'a@example.test' } }));
    expect(calls[0]!.headers.get('X-Powermove-Client')).toBe('desktop/0.9.0');
    expect(calls[0]!.headers.get('Authorization')).toBeNull();
    expect(JSON.parse(String(calls[0]!.init.body))).toEqual({ email: 'a@example.test' });
  });

  it('throws ApiError with the parsed body on a non-2xx answer', async () => {
    const { fetch } = mockFetch(() => json({ error: 'handle_taken', detail: 'taken' }, 409));
    const client = createCloudClient({ origin: ORIGIN, getToken: () => 'tok', appVersion: '1.0.0', fetch });
    const args = { json: { handle: 'jude' } };
    const error = await client.request(Me.SetHandle.Res, (api) => api.v1.me.handle.$post(args)).catch((caught: unknown) => caught);
    expect(error).toBeInstanceOf(ApiError);
    expect((error as ApiError).body).toEqual({ error: 'handle_taken', detail: 'taken' });
    expect((error as ApiError).status).toBe(409);
  });

  it('parses code-specific error fields', async () => {
    const { fetch } = mockFetch(() => json({ error: 'client_too_old', minimum: '2.0.0' }, 426));
    const client = createCloudClient({ origin: ORIGIN, getToken: () => null, appVersion: '1.0.0', fetch });
    const error = await client.request(Me.Get.Res, (api) => api.v1.me.$get()).catch((caught: unknown) => caught);
    expect((error as ApiError).body).toEqual({ error: 'client_too_old', minimum: '2.0.0' });
  });

  it('turns unparsable bodies and network failures into internal errors', async () => {
    const html = createCloudClient({ origin: ORIGIN, getToken: () => null, appVersion: '1.0.0', fetch: mockFetch(() => new Response('<html>bad gateway</html>', { status: 502 })).fetch });
    expect(((await html.request(Me.Get.Res, (api) => api.v1.me.$get()).catch((e: unknown) => e)) as ApiError).body.error).toBe('internal');

    const unknownCode = createCloudClient({ origin: ORIGIN, getToken: () => null, appVersion: '1.0.0', fetch: mockFetch(() => json({ error: 'teapot' }, 418)).fetch });
    expect(((await unknownCode.request(Me.Get.Res, (api) => api.v1.me.$get()).catch((e: unknown) => e)) as ApiError).body.error).toBe('internal');

    const wrongShape = createCloudClient({ origin: ORIGIN, getToken: () => null, appVersion: '1.0.0', fetch: mockFetch(() => json({ user: 'nope' })).fetch });
    expect(((await wrongShape.request(Me.Get.Res, (api) => api.v1.me.$get()).catch((e: unknown) => e)) as ApiError).body.error).toBe('internal');

    const offline = createCloudClient({ origin: ORIGIN, getToken: () => null, appVersion: '1.0.0', fetch: (async () => { throw new TypeError('fetch failed'); }) as typeof globalThis.fetch });
    const failure = (await offline.request(Me.Get.Res, (api) => api.v1.me.$get()).catch((e: unknown) => e)) as ApiError;
    expect(failure).toBeInstanceOf(ApiError);
    expect(failure.body.error).toBe('internal');
  });

  it('accepts an empty 204 when no body is expected', async () => {
    const { calls, fetch } = mockFetch(() => new Response(null, { status: 204 }));
    const client = createCloudClient({ origin: ORIGIN, getToken: () => 'tok', appVersion: '1.0.0', fetch });
    await expect(client.request(null, (api) => api.v1.me.$delete())).resolves.toBeUndefined();
    expect(calls[0]!.init.method).toBe('DELETE');
  });

  it('never sends the bearer to any origin but the configured one', async () => {
    const { calls, fetch } = mockFetch(() => json(ME));
    const bound = createBoundFetch({ origin: ORIGIN, getToken: () => 'secret-token', appVersion: '1.0.0', fetch });
    await bound(`${ORIGIN}/v1/me`);
    await bound('https://elsewhere.example.test/v1/me', { headers: { Authorization: 'Bearer secret-token' } });
    await bound('https://cloud.example.test.evil.test/v1/me');
    await bound('http://cloud.example.test/v1/me');
    await bound(new Request('https://elsewhere.example.test/x', { headers: { Authorization: 'Bearer leaked' } }));
    expect(calls.map((call) => call.headers.get('Authorization'))).toEqual(['Bearer secret-token', null, null, null, null]);
    expect(calls.every((call) => call.headers.get('X-Powermove-Client') === 'desktop/1.0.0')).toBe(true);
  });

  it('replaces a forged Authorization header with the session token', async () => {
    const { calls, fetch } = mockFetch(() => json(ME));
    const client = createCloudClient({ origin: ORIGIN, getToken: () => 'secret-token', appVersion: '1.0.0', fetch });
    await client.request(Me.Get.Res, (api) => api.v1.me.$get(undefined, { init: { headers: { Authorization: 'Bearer forged' } } }));
    expect(calls[0]!.headers.get('Authorization')).toBe('Bearer secret-token');
  });
});
