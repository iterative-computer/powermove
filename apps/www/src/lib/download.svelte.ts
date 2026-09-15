import { RELEASES, RELEASES_REPO } from './links';

type Asset = { name: string; browser_download_url: string };
type Release = { draft: boolean; tag_name: string; assets: Asset[] };

/** Direct link to the newest macOS build. Falls back to the releases page until resolved. */
export const download = $state({ href: RELEASES, version: '' });

let started = false;

export async function resolveDownload() {
  if (started || typeof fetch === 'undefined') return;
  started = true;
  try {
    const res = await fetch(`https://api.github.com/repos/${RELEASES_REPO}/releases?per_page=10`, {
      headers: { Accept: 'application/vnd.github+json' },
    });
    if (!res.ok) return;
    const releases = (await res.json()) as Release[];
    const arm = /arm64|aarch64|apple/i;
    for (const r of releases) {
      if (r.draft) continue;
      const dmgs = r.assets.filter((a) => a.name.endsWith('.dmg'));
      if (!dmgs.length) continue;
      const pick = dmgs.find((a) => arm.test(a.name)) ?? dmgs[0];
      download.href = pick.browser_download_url;
      download.version = r.tag_name.replace(/^v/, '');
      return;
    }
  } catch {
    /* keep the releases page */
  }
}
