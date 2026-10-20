import { expect, test } from './helpers/app';
import { installStoreExtensions, waitForCommands } from './helpers/sandbox';

const PNG = 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII=';
const REMOTE_URL = 'https://images.example.com/waiting.png';

test('a delayed extension URL import cannot land in a different project', async ({ session }) => {
  await installStoreExtensions(session, [{ fixture: 'sandbox-links' }]);
  await session.relaunch();
  await session.openEditor();
  await waitForCommands(session, ['sandbox-links'], 'import');
  const { page, app } = session;
  const original = await page.evaluate(() => {
    const PM = (window as any).PM;
    return { id: PM.proj.id, assets: Object.keys(PM.proj.assets ?? {}) };
  });
  expect(original.assets).toEqual([]);

  // Only main's DNS/socket boundary is replaced. The sandbox, real native
  // download file, IPC chunk reads, image decoding and project import all run.
  await app.evaluate((_electron, png) => {
    const state = { entered: false, release: null as (() => void) | null, requests: [] as string[], released: [] as string[] };
    const gate = new Promise<void>(resolve => { state.release = resolve; });
    (globalThis as any).__remoteOwnership = state;
    const media = (globalThis as any).__powermoveRemoteMedia;
    const release = media.release.bind(media);
    media.release = async (owner: number, token: unknown) => {
      state.released.push(String(token));
      return release(owner, token);
    };
    media.useNetworkForTests({
      resolve: async () => [{ address: '93.184.215.14', family: 4 }],
      transport: async ({ url }: { url: URL }) => {
        state.requests.push(url.href);
        state.entered = true;
        await gate;
        const bytes = Uint8Array.from(atob(png), char => char.charCodeAt(0));
        return { status: 200, headers: {}, cancel() {}, body: (async function* () { yield bytes; })() };
      }
    });
  }, PNG);

  const importUrl = () => page.evaluate(url => (window as any).PM.Kernel.commands.get('sandbox-links.import').run(url), REMOTE_URL);
  const pending = importUrl();
  await expect.poll(() => app.evaluate(() => (globalThis as any).__remoteOwnership.entered)).toBe(true);
  const nextId = await page.evaluate(() => {
    const PM = (window as any).PM;
    const project = PM.mkProject({ name: 'After remote import' });
    window.dispatchEvent(new CustomEvent('pm-open-project', { detail: project }));
    return project.id;
  });
  expect(nextId).not.toBe(original.id);
  await expect.poll(() => page.evaluate(() => (window as any).PM.proj.id)).toBe(nextId);
  await app.evaluate(() => (globalThis as any).__remoteOwnership.release());
  expect(await pending).toMatchObject({ error: expect.stringContaining('switched projects'), code: 'STALE_MEDIA_IMPORT' });
  expect(await page.evaluate(() => Object.keys((window as any).PM.proj.assets ?? {}))).toEqual([]);
  expect(await app.evaluate(() => (globalThis as any).__remoteOwnership.released.length)).toBe(1);

  // The initiating document also remains unchanged, and an import started in
  // the current document still succeeds through the same native pipeline.
  expect(await page.evaluate(id => (window as any).PM.Tabs.activate(id), original.id)).toBe(true);
  expect(await page.evaluate(() => Object.keys((window as any).PM.proj.assets ?? {}))).toEqual([]);
  expect(await page.evaluate(id => (window as any).PM.Tabs.activate(id), nextId)).toBe(true);
  const imported = await importUrl();
  expect(imported.error).toBeUndefined();
  expect(typeof imported.ok).toBe('string');
  expect(await page.evaluate(id => {
    const asset = (window as any).PM.proj.assets[id];
    return { name: asset.name, kind: asset.kind };
  }, imported.ok)).toEqual({ name: 'waiting.png', kind: 'image' });
  expect(await app.evaluate(() => (globalThis as any).__remoteOwnership.requests)).toEqual([REMOTE_URL, REMOTE_URL]);
  expect(await app.evaluate(() => (globalThis as any).__remoteOwnership.released.length)).toBe(2);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
