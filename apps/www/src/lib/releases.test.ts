import { describe, test } from 'node:test';
import assert from 'node:assert/strict';
import { fetchDownload } from './releases';

function mockFetch(body: unknown, status = 200) {
  return (async () => new Response(JSON.stringify(body), { status })) as typeof fetch;
}
const asset = (name: string) => ({ name, browser_download_url: `https://example.com/${name}` });

describe('macOS download metadata', () => {
  test('skips drafts and releases without DMGs, preferring Apple Silicon', async () => {
    const result = await fetchDownload(mockFetch([
      { draft: true, tag_name: 'v3', assets: [asset('draft.dmg')] },
      { draft: false, tag_name: 'v2', assets: [asset('app.zip')] },
      { draft: false, tag_name: 'v1.0.0', assets: [asset('intel.dmg'), asset('mac-arm64.dmg')] },
    ]));
    assert.deepEqual(result, { href: 'https://example.com/mac-arm64.dmg', version: '1.0.0' });
  });

  test('falls back to an available DMG', async () => {
    assert.deepEqual(await fetchDownload(mockFetch([
      { draft: false, tag_name: '1.0.0', assets: [asset('universal.dmg')] },
    ])), { href: 'https://example.com/universal.dmg', version: '1.0.0' });
  });

  test('rejects failed and empty lookups so a build cannot publish an empty label', async () => {
    await assert.rejects(fetchDownload(mockFetch({}, 403)), /403/);
    await assert.rejects(fetchDownload(mockFetch([])), /No macOS release/);
  });
});
