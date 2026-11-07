import { it, expect } from 'vitest';
import { mkdtemp, mkdir, writeFile, readFile, rm, stat } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { figmaPluginSource, installFigmaPlugin } from './figma-plugin';
it('resolves development and packaged resource roots', () => {
  expect(figmaPluginSource('/repo/apps/desktop', '/app/Resources', false)).toBe('/repo/packages/figma-plugin');
  expect(figmaPluginSource('/app/Resources/app.asar', '/app/Resources', true)).toBe('/app/Resources/figma-plugin');
});
it('installs to a stable path, refreshes changed bundles and repairs missing files', async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), 'pm-figma-resource-test-')),
    bundle = path.join(root, 'Resources/figma-plugin'),
    userData = path.join(root, 'userData');
  try {
    await mkdir(path.join(bundle, 'dist'), { recursive: true });
    for (const name of ['manifest.json', 'ui.html', 'dist/code.js']) await writeFile(path.join(bundle, name), name);
    const manifest = await installFigmaPlugin(bundle, userData);
    expect(manifest).toBe(path.join(userData, 'Figma Plugin/manifest.json'));
    const before = await stat(manifest);
    expect(await installFigmaPlugin(bundle, userData)).toBe(manifest);
    expect((await stat(manifest)).mtimeMs).toBe(before.mtimeMs);
    await writeFile(path.join(bundle, 'dist/code.js'), 'new version');
    await installFigmaPlugin(bundle, userData);
    expect(await readFile(path.join(userData, 'Figma Plugin/dist/code.js'), 'utf8')).toBe('new version');
    await rm(manifest);
    await installFigmaPlugin(bundle, userData);
    expect(await readFile(manifest, 'utf8')).toBe('manifest.json');
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

it('ships only the public-API plugin and builds it before both desktop compilation and packaging', async () => {
  const config = await readFile(new URL('../../electron-builder.yml', import.meta.url), 'utf8');
  const manifest = JSON.parse(
    await readFile(new URL('../../../../packages/figma-plugin/manifest.json', import.meta.url), 'utf8'),
  );
  expect(config).toContain('to: figma-plugin');
  expect(config).toContain('beforePack: ./scripts/before-pack.mjs');
  expect(config).toContain('- dist/code.js');
  expect(manifest.enablePrivatePluginApi).toBeUndefined();
  // Import succeeds without an ID, but real Figma then rejects clientStorage.
  expect(manifest.id).toBe('powermove-local');
  expect(manifest.networkAccess.allowedDomains).toEqual(['http://localhost:47832']);
  expect(manifest.networkAccess.reasoning.length).toBeGreaterThan(10);
  expect(await readFile(new URL('../../electron.vite.config.ts', import.meta.url), 'utf8')).toContain(
    'bundle-figma-plugin',
  );
});
