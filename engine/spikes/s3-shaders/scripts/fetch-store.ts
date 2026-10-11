/*
 * Downloads every public Store extension release (read-only routes the desktop
 * Store client uses: GET /v1/store/extensions, /v1/store/x/:handle/:slug,
 * /v1/store/releases/:id/tree, and the per-file route). Sources are third-party
 * code, so they land in work/store/ (gitignored) and are never committed.
 *
 * Usage: bun scripts/fetch-store.ts [--origin https://cloud.trypowermove.com]
 */
import { mkdir, writeFile } from 'node:fs/promises';
import { dirname, join } from 'node:path';

const args = process.argv.slice(2);
const origin = args.includes('--origin') ? args[args.indexOf('--origin') + 1]! : 'https://cloud.trypowermove.com';
const root = join(import.meta.dir, '..', 'work', 'store');

async function json<T>(path: string): Promise<T> {
  const response = await fetch(origin + path, { headers: { accept: 'application/json' } });
  if (!response.ok) throw new Error(`${path}: HTTP ${response.status}`);
  return (await response.json()) as T;
}

interface Listing { owner: { handle: string }; slug: string; category: string; latest: { id: string; version: string } | null }
interface Release { id: string; version: string; yankedAt: string | null; apiVersion: number }
interface Detail { releases: Release[] }
interface Tree { files: Array<{ path: string; size: number }> }

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
    const dir = join(root, listing.owner.handle, listing.slug, release.version);
    for (const file of tree.files) {
      if (!/\.(ts|js|mjs|json|md)$/.test(file.path)) continue;
      const url = `${origin}/v1/store/x/${encodeURIComponent(listing.owner.handle)}/${encodeURIComponent(listing.slug)}/r/${encodeURIComponent(release.version)}/file/${file.path.split('/').map(encodeURIComponent).join('/')}`;
      const response = await fetch(url);
      if (!response.ok) throw new Error(`${url}: HTTP ${response.status}`);
      const target = join(dir, file.path);
      await mkdir(dirname(target), { recursive: true });
      await writeFile(target, new Uint8Array(await response.arrayBuffer()));
    }
    index.push({ handle: listing.owner.handle, slug: listing.slug, category: listing.category, version: release.version, releaseId: release.id, apiVersion: release.apiVersion, yanked: !!release.yankedAt, files: tree.files.map((f) => f.path), latest: listing.latest?.id === release.id });
  }
}
await mkdir(root, { recursive: true });
await writeFile(join(root, 'index.json'), JSON.stringify({ origin, fetchedAt: new Date().toISOString(), releases: index }, null, 2));
console.log(`fetched ${index.length} releases of ${listings.length} public extensions into ${root}`);
