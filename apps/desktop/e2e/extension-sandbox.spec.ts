import { cp, mkdir, readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { expect, test } from './helpers/app';

const origin = { repoId: 'external-repo', releaseId: 'release-1', coordinate: 'someone/sandbox', version: '1.0.0', treeSha: 'tree', commitSha: 'commit', ownerPublisherId: 'someone-else' };

test('store code runs in an opaque iframe and network permission controls fetch', async ({ session }) => {
  const extensions = path.join(session.userData, 'extensions');
  await mkdir(extensions, { recursive: true });
  const fixture = path.resolve('test/fixtures/sandboxed-ext');
  await cp(fixture, path.join(extensions, 'sandboxed-ext'), { recursive: true });
  const denied = path.join(extensions, 'sandbox-no-network');
  await cp(fixture, denied, { recursive: true });
  const manifest = JSON.parse(await readFile(path.join(denied, 'manifest.json'), 'utf8'));
  manifest.id = 'sandbox-no-network';
  manifest.permissions = ['project:write'];
  await writeFile(path.join(denied, 'manifest.json'), JSON.stringify(manifest));
  await writeFile(path.join(session.userData, 'extensions-provenance.json'), JSON.stringify({
    'sandboxed-ext': { localId: 'sandboxed-ext', envKey: 'external-repo', origin },
    'sandbox-no-network': { localId: 'sandbox-no-network', envKey: 'external-repo-2', origin: { ...origin, repoId: 'external-repo-2' } }
  }));
  await session.relaunch();
  await session.openEditor();
  const { page } = session;
  await page.waitForFunction(() => (window as any).PM?.Kernel?.effects?.has('sandbox-tint'));
  await page.waitForFunction(() => document.querySelectorAll('iframe[src*="/host/ext-sandbox.html"]').length === 2);
  // Sandboxed frames are out-of-process opaque origins; Playwright's Electron
  // driver cannot evaluate inside them, so each fixture publishes its own proof
  // as effect data (see test/fixtures/sandboxed-ext/index.ts) and we read it here.
  const frames = page.frames().filter(frame => frame.url().includes('/host/ext-sandbox.html'));
  expect(frames).toHaveLength(2);
  for (const frame of frames) {
    const params = new URL(frame.url()).searchParams;
    expect(params.get('id')).toMatch(/^sandbox/);
  }
  await page.waitForFunction(() => {
    const list = (window as any).PM?.Kernel?.effects?.list?.() ?? [];
    return list.filter((e: any) => String(e.id).startsWith('sandbox-proof-')).length === 2;
  }, undefined, { timeout: 15_000 });
  const checks = await page.evaluate(() => {
    const list = (window as any).PM?.Kernel?.effects?.list?.() ?? [];
    return list.filter((e: any) => String(e.id).startsWith('sandbox-proof-'))
      .map((e: any) => ({ id: String(e.id).slice('sandbox-proof-'.length), ...JSON.parse(e.label) }));
  });
  console.log('SANDBOX PROOF', JSON.stringify(checks));
  expect(checks).toContainEqual({ id: 'sandboxed-ext', powermove: false, parentDenied: true, cspBlocked: false });
  expect(checks).toContainEqual({ id: 'sandbox-no-network', powermove: false, parentDenied: true, cspBlocked: true });
});
