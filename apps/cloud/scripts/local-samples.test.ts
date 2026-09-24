import { expect, test } from 'bun:test';
import { cp, mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { basename, join } from 'node:path';
import { parseManifest, type ExtensionManifest } from '@powermove/registry/manifest';
import { walkDir } from '@powermove/registry/node';
import { scanCapabilities } from '@powermove/registry/scan';
import { createApp } from '../src/app';
import { extensions, releases, repos } from '../src/db/schema';
import { withData } from '../test/db';
import { makeEnv } from '../test/env';
import { publisher } from '../test/publish-fixture';
import { publishLocalSamples, sampleDirs, sampleListings } from './local-samples';

const decoder = new TextDecoder('utf-8', { fatal: true });
const expected = ['glass-tint', 'ease-lab', 'colour-match', 'wipe-set'];

test('all sample trees compile for the desktop sandbox and declare scanned capabilities', async () => {
  // Keep desktop's Electron-only TypeScript dependencies outside cloud's tsconfig.
  const { compileExtension } = await import('../../desktop/src/main/extensions/' + 'compiler') as {
    compileExtension(options: { dir: string; entry: string; outDir: string }): Promise<{ ok: true; bundlePath: string } | { ok: false; error: string }>;
  };
  const temporary = await mkdtemp(join(tmpdir(), 'powermove-samples-'));
  try {
    for (const dir of sampleDirs(true)) {
      const files = await walkDir(dir);
      const raw = JSON.parse(decoder.decode(files.find((file) => file.path === 'manifest.json')!.bytes));
      const parsed = parseManifest(raw);
      expect(parsed.ok).toBe(true);
      if (!parsed.ok) continue;
      const manifest = parsed.manifest;
      expect(manifest.apiVersion).toBe(3);
      expect(manifest.author).toBe('user');
      const findings = scanCapabilities(files.map((file) => ({ path: file.path, text: decoder.decode(file.bytes) })));
      expect(findings.filter((finding) => !manifest.permissions?.includes(finding.capability))).toEqual([]);

      // The desktop compiler uses the directory basename as the extension id.
      // Versioned sample sources therefore need a temporary slug-named copy.
      const compileDir = join(temporary, `${basename(dir)}-source`, manifest.id);
      await cp(dir, compileDir, { recursive: true });
      const result = await compileExtension({ dir: compileDir, entry: manifest.entry ?? 'index.ts', outDir: join(temporary, `${basename(dir)}-output`) });
      if (!result.ok) throw new Error(`${basename(dir)}: ${result.error}`);
      expect(result.ok).toBe(true);
    }
  } finally {
    await rm(temporary, { recursive: true, force: true });
  }
});

test('publishes four public Mara listings, skips repeats, then offers Glass Tint 1.1.0', () =>
  withData(async (data) => {
    const env = makeEnv(data);
    const actor = await publisher(data, env, 'mara');
    const app = createApp({ data: () => data });
    const fetcher = async (input: string | URL | Request, init?: RequestInit) =>
      app.request(new URL(String(input)).pathname, init, env);
    const options = { fetch: fetcher, token: actor.token, origin: 'https://cloud.test' };

    const initial = await publishLocalSamples(options);
    expect(initial.map((plan) => plan.coordinate)).toEqual(expected.map((slug) => `mara/${slug}`));
    expect(initial.every((plan) => plan.status === 'published')).toBe(true);
    expect((await publishLocalSamples(options)).every((plan) => plan.status === 'skipped')).toBe(true);

    const listingRows = await data.db.select().from(extensions);
    const repoRows = await data.db.select().from(repos);
    expect(listingRows).toHaveLength(4);
    expect(repoRows).toHaveLength(4);
    expect(repoRows.every((row) => row.ownerId === actor.publisher.id && row.visibility === 'public')).toBe(true);
    for (const listing of listingRows) {
      const expectedListing = sampleListings[listing.slug]!;
      expect(listing).toMatchObject({ handle: 'mara', name: expectedListing.name, tagline: expectedListing.tagline, category: expectedListing.category, licence: 'MIT' });
    }
    expect(listingRows.find((row) => row.slug === 'ease-lab')?.permissions).toEqual(['project:write']);
    expect(listingRows.find((row) => row.slug === 'colour-match')?.permissions).toEqual(['network']);

    const update = await publishLocalSamples({ ...options, update: true });
    expect(update.find((plan) => plan.version === '1.1.0')).toMatchObject({ coordinate: 'mara/glass-tint', status: 'published' });
    expect(update.filter((plan) => plan.status === 'skipped')).toHaveLength(4);
    expect((await publishLocalSamples({ ...options, update: true })).every((plan) => plan.status === 'skipped')).toBe(true);
    const releaseRows = await data.db.select().from(releases);
    expect(releaseRows).toHaveLength(5);
    const glassUpdate = releaseRows.find((row) => (row.manifest as ExtensionManifest).id === 'glass-tint' && row.version === '1.1.0');
    expect(glassUpdate?.notes).toContain('Edge control');
    const colour = releaseRows.find((row) => (row.manifest as ExtensionManifest).id === 'colour-match');
    expect((colour?.manifest as ExtensionManifest).vars).toEqual([
      { key: 'OPENAI_API_KEY', label: 'OpenAI API key', secret: true, required: true, hint: 'Used for the colour mapping model.' },
      { key: 'PALETTE_SIZE', label: 'Palette size', hint: 'Defaults to 5.' },
    ]);
  }));
