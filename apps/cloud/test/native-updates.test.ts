import { afterEach, beforeEach, describe, expect, test } from 'bun:test';
import { createApp } from '../src/app';
import { nativeUpdateResponse } from '../src/routes/native-updates';
import { FakeR2 } from './r2';

const versions = new Map([
  ['native/latest/version', '2.3.4\n'],
  ['native/2.3.4/appcast.xml', '<rss>fixture appcast</rss>'],
  ['native/2.3.4/policy.json', '{"payload":"signed fixture"}'],
  ['native/2.3.4/Powermove-Native-2.3.4-mac-arm64.zip', 'zip'],
  ['native/2.3.4/Powermove-Native-2.3.4-mac-arm64.dmg', 'dmg'],
  ['tars/private.tar.gz', 'registry tar'],
]);

let bucket: FakeR2;
beforeEach(async () => {
  bucket = new FakeR2();
  for (const [key, contents] of versions) await bucket.put(key, contents);
});

let server: ReturnType<typeof Bun.serve> | undefined;
afterEach(() => { server?.stop(true); server = undefined; });

describe('native update downloads', () => {
  test('serves both files for the active version directly over HTTP', async () => {
    const app = createApp({ data: () => { throw new Error('native update route must not access the DB'); } });
    server = Bun.serve({
      hostname: '127.0.0.1', port: 0,
      fetch: request => app.fetch(request, { TARS: bucket } as unknown as CloudflareBindings),
    });
    for (const [asset, contentType, body] of [
      ['appcast.xml', 'application/rss+xml', '<rss>fixture appcast</rss>'],
      ['policy.json', 'application/json', '{"payload":"signed fixture"}'],
    ]) {
      const result = await fetch(new URL(`/native/latest/${asset}`, server.url), { redirect: 'manual' });
      expect(result.status).toBe(200);
      expect(result.headers.get('location')).toBeNull();
      expect(result.headers.get('content-type')).toContain(contentType!);
      expect(result.headers.get('cache-control')).toBe('no-store');
      expect(await result.text()).toBe(body!);
    }
  });

  test('serves versioned archives and DMGs with no access to registry keys', async () => {
    const request = (path: string) => new Request(`https://cloud.trypowermove.com${path}`);
    const result = await nativeUpdateResponse(
      request('/native/2.3.4/Powermove-Native-2.3.4-mac-arm64.dmg'), bucket,
    );
    expect(result?.status).toBe(200);
    expect(result?.headers.get('cache-control')).toContain('immutable');
    expect(await result?.text()).toBe('dmg');
    const head = await nativeUpdateResponse(new Request('https://cloud.trypowermove.com/native/2.3.4/Powermove-Native-2.3.4-mac-arm64.dmg', { method: 'HEAD' }), bucket);
    expect(head?.headers.get('content-length')).toBe('3');
    expect(head?.body).toBeNull();
    expect(await nativeUpdateResponse(request('/v1/store/download'), bucket)).toBeNull();
    for (const path of ['/native/tars/private.tar.gz', '/native/2.3.4/Powermove-Native-4.0.0-mac-arm64.zip',
                        '/native/2.3.4/appcast.xml', '/native/latest/unknown.json']) {
      expect((await nativeUpdateResponse(request(path), bucket))?.status).toBe(404);
    }
  });
});
