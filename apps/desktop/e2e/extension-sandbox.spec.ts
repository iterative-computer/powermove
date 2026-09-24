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
  manifest.permissions = [];
  await writeFile(path.join(denied, 'manifest.json'), JSON.stringify(manifest));
  await writeFile(path.join(session.userData, 'extensions-provenance.json'), JSON.stringify({
    'sandboxed-ext': { localId: 'sandboxed-ext', envKey: 'external-repo', origin },
    'sandbox-no-network': { localId: 'sandbox-no-network', envKey: 'external-repo-2', origin: { ...origin, repoId: 'external-repo-2' } }
  }));
  await session.relaunch();
  await session.openEditor();
  const { page } = session;
  await page.waitForFunction(() => (window as any).PM?.Kernel?.effects?.has('sandboxed-ext-tint'));
  // Two runtime iframes (the fixture's panel adds a view iframe per extension, excluded here).
  await page.waitForFunction(() => [...document.querySelectorAll('iframe[src*="/host/ext-sandbox.html"]')]
    .filter((f) => !(f as HTMLIFrameElement).src.includes('&view=')).length === 2);
  // Sandboxed frames are out-of-process opaque origins; Playwright's Electron
  // driver cannot evaluate inside them, so each fixture publishes its own proof
  // as effect data (see test/fixtures/sandboxed-ext/index.ts) and we read it here.
  const frames = page.frames().filter(frame => frame.url().includes('/host/ext-sandbox.html') && !frame.url().includes('&view='));
  expect(frames).toHaveLength(2);
  for (const frame of frames) {
    const params = new URL(frame.url()).searchParams;
    expect(params.get('id')).toMatch(/^sandbox/);
  }
  await page.waitForFunction(() => {
    const list = (window as any).PM?.Kernel?.effects?.list?.() ?? [];
    return list.filter((e: any) => String(e.id).endsWith('-proof')).length === 2;
  }, undefined, { timeout: 15_000 });
  const checks = await page.evaluate(() => {
    const list = (window as any).PM?.Kernel?.effects?.list?.() ?? [];
    return list.filter((e: any) => String(e.id).endsWith('-proof'))
      .map((e: any) => ({ id: String(e.id).slice(0, -'-proof'.length), ...JSON.parse(e.label) }));
  });
  console.log('SANDBOX PROOF', JSON.stringify(checks));
  expect(checks).toContainEqual({ id: 'sandboxed-ext', powermove: false, parentDenied: true, cspBlocked: false, deleteRefused: false });
  expect(checks).toContainEqual({ id: 'sandbox-no-network', powermove: false, parentDenied: true, cspBlocked: true, deleteRefused: true });
});

test('a store extension’s Svelte panel renders in its own view iframe with host chrome and keys', async ({ session }, testInfo) => {
  const extensions = path.join(session.userData, 'extensions');
  await mkdir(extensions, { recursive: true });
  await cp(path.resolve('test/fixtures/sandboxed-ext'), path.join(extensions, 'sandboxed-ext'), { recursive: true });
  await writeFile(path.join(session.userData, 'extensions-provenance.json'), JSON.stringify({
    'sandboxed-ext': { localId: 'sandboxed-ext', envKey: 'external-repo', origin }
  }));
  await session.relaunch();
  await session.openEditor();
  const { page } = session;
  const panelId = 'sandboxed-ext-panel';
  await page.waitForFunction((id) => (window as any).PM?.Kernel?.panels?.has(id), panelId);
  await page.evaluate((id) => {
    const PM = (window as any).PM;
    PM.WS.mutate((workspace: any) => PM.Layout.addPanel(workspace, id, 'right'));
  }, panelId);

  const panel = page.locator(`#panel-${panelId}`);
  await expect(panel).toBeVisible();
  await expect(panel).toHaveClass(/\bframe\b/);
  await expect(panel.locator('header .ptitle')).toHaveText('Sandbox panel');
  const frame = panel.locator('iframe.ext-panel-frame');
  await expect(frame).toHaveAttribute('sandbox', 'allow-scripts');
  await expect(frame).toHaveAttribute('src', new RegExp(`/host/ext-sandbox\\.html\\?id=sandboxed-ext&view=${panelId}&perms=`));
  // The view reports its own mount; Playwright cannot look inside the frame.
  await expect(frame).toHaveAttribute('data-state', 'ready', { timeout: 15_000 });
  const viewFrames = page.frames().filter(f => f.url().includes(`view=${panelId}`));
  expect(viewFrames).toHaveLength(1);

  // Moving the panel to another dock keeps its view document alive (no reload).
  const loads = await frame.getAttribute('data-loads');
  await page.evaluate((id) => {
    const PM = (window as any).PM;
    PM.WS.mutate((workspace: any) => PM.Layout.addPanel(workspace, id, 'left'));
  }, panelId);
  await expect.poll(() => page.evaluate((id) => document.getElementById(`panel-${id}`)?.closest('[data-dock]')?.getAttribute('data-dock'), panelId)).toBe('left');
  await page.waitForTimeout(300);
  expect(await frame.getAttribute('data-loads')).toBe(loads);
  await expect(frame).toHaveAttribute('data-state', 'ready');

  // Only the extension's own bindings cross from its view; a field keeps its own keys.
  const keyProofCount = () => page.evaluate(() => ((window as any).PM?.Kernel?.effects?.list?.() ?? [])
    .filter((effect: { id: string }) => effect.id.startsWith('sandboxed-ext-key-')).length);
  const box = (await frame.boundingBox())!;
  await page.mouse.click(box.x + box.width / 2, box.y + box.height - 8); // empty panel area
  await expect.poll(() => page.evaluate(() => document.activeElement?.className)).toBe('ext-panel-frame');
  await page.keyboard.press('Meta+Shift+9');
  await page.keyboard.press('g');
  await expect.poll(keyProofCount).toBe(2);
  // Click the Note field (third row of the fixture) and type: `g` stays in the field.
  await page.mouse.click(box.x + 40, box.y + 12 + 26 * 2 + 12 + 14);
  await page.keyboard.press('g');
  await page.keyboard.press('Meta+Shift+9');
  await page.waitForTimeout(200);
  expect(await keyProofCount()).toBe(2);
  await page.keyboard.press('Escape');

  for (const scheme of ['dark', 'light'] as const) {
    await page.evaluate((value) => (window as any).PM.theme.apply(value), scheme);
    await page.waitForTimeout(300); // one theme push to the view, then paint
    const shot = await panel.screenshot();
    await testInfo.attach(`sandbox-panel-${scheme}`, { body: shot, contentType: 'image/png' });
    await writeFile(testInfo.outputPath(`sandbox-panel-${scheme}.png`), shot);
  }

  // The Library shows the icon on the extension's art rather than a DOM clone of the frame.
  await page.evaluate(() => (window as any).PM.LibraryUI.open());
  const card = page.locator(`[data-panel-id="${panelId}"]`);
  await expect(card.locator('.library-live.is-sandboxed.has-art')).toBeVisible();
  await expect(card.locator('iframe')).toHaveCount(0);
  const libraryShot = await card.screenshot();
  await testInfo.attach('sandbox-panel-library', { body: libraryShot, contentType: 'image/png' });
  await writeFile(testInfo.outputPath('sandbox-panel-library.png'), libraryShot);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
