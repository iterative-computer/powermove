/*
 * Downloads every public Store extension release through the read-only
 * routes the desktop Store client uses without signing in (GET
 * /v1/store/extensions, /v1/store/x/:handle/:slug,
 * /v1/store/releases/:id/tree, and the per-file route), as spike S3 did.
 * Store sources are third-party code: they are written outside the
 * repository and never executed by the shader gate (`collect.ts` reads
 * their registrations statically).
 *
 * Usage: bun engine/crates/pm-render/tools/shaders/fetch-store.ts [--origin https://cloud.trypowermove.com] [--out <dir>]
 * Default output: ~/Library/Application Support/Powermove Engine Corpus/store-<UTC stamp>/
 */
import { mkdir, writeFile } from 'node:fs/promises';
import { homedir } from 'node:os';
import { dirname, join, resolve, sep } from 'node:path';

import { REPO } from '../effects/preambles';

const args = process.argv.slice(2);
const origin = args.includes('--origin') ? args[args.indexOf('--origin') + 1]! : 'https://cloud.trypowermove.com';
const stamp = new Date().toISOString().replace(/[-:]/g, '').replace(/\..*/, '').replace('T', '-');
const root = args.includes('--out')
  ? resolve(args[args.indexOf('--out') + 1]!)
  : join(homedir(), 'Library', 'Application Support', 'Powermove Engine Corpus', `store-${stamp}`);
if (root.startsWith(resolve(REPO) + sep)) throw new Error('Store sources are never written inside the repository');

async function json<T>(path: string): Promise<T> {
  const response = await fetch(origin + path, { headers: { accept: 'application/json' } });
  if (!response.ok) throw new Error(`${path}: HTTP ${response.status}`);
  return (await response.json()) as T;
}

interface Listing { owner: { handle: string }; slug: string; category: string; latest: { id: string; version: string } | null }
interface Release { id: string; version: string; yankedAt: string | null; apiVersion: number }
interface Detail { releases: Release[] }
interface Tree { files: Array<{ path: string; size: number }> }

const safe = (part: string) => {
  if (!/^[A-Za-z0-9._-]+$/.test(part) || part === '.' || part === '..') throw new Error(`unsafe path segment ${JSON.stringify(part)}`);
  return part;
};

const listings: Listing[] = [];
let cursor: string | null = null;
do {
  const page: { items: Listing[]; nextCursor: string | null } = await json(`/v1/store/extensions?limit=50&sort=new${cursor ? `&cursor=${encodeURIComponent(cursor)}` : ''}`);
  listings.push(...page.items);
  cursor = page.nextCursor;
} while (cursor);

const index: unknown[] = [];
for (const listing of listings) {
  const detail = await json<Detail>(`/v1/store/x/${encodeURIComponent(listing.owner.handle)}/${encodeURIComponent(listing.slug)}`);
  for (const release of detail.releases) {
    const tree = await json<Tree>(`/v1/store/releases/${release.id}/tree`);
    const dir = join(root, safe(listing.owner.handle), safe(listing.slug), safe(release.version));
    for (const file of tree.files) {
      if (!/\.(ts|js|mjs|json|md)$/.test(file.path)) continue;
      const parts = file.path.split('/').map(safe);
      const url = `${origin}/v1/store/x/${encodeURIComponent(listing.owner.handle)}/${encodeURIComponent(listing.slug)}/r/${encodeURIComponent(release.version)}/file/${parts.map(encodeURIComponent).join('/')}`;
      const response = await fetch(url);
      if (!response.ok) throw new Error(`${url}: HTTP ${response.status}`);
      const target = join(dir, ...parts);
      await mkdir(dirname(target), { recursive: true });
      await writeFile(target, new Uint8Array(await response.arrayBuffer()));
    }
    index.push({ handle: listing.owner.handle, slug: listing.slug, category: listing.category, version: release.version, releaseId: release.id, apiVersion: release.apiVersion, yanked: !!release.yankedAt, files: tree.files.map((f) => f.path), latest: listing.latest?.id === release.id });
  }
}
await mkdir(root, { recursive: true });
await writeFile(join(root, 'index.json'), JSON.stringify({ origin, fetchedAt: new Date().toISOString(), releases: index }, null, 2));
console.log(`fetched ${index.length} releases of ${listings.length} public extensions into ${root}`);
