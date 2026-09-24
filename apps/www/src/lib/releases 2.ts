import { RELEASES_REPO } from './links';

type Asset = { name: string; browser_download_url: string };
type Release = { draft: boolean; tag_name: string; assets: Asset[] };
export type Download = { href: string; version: string };

export async function fetchDownload(fetcher: typeof fetch): Promise<Download> {
  const response = await fetcher(`https://api.github.com/repos/${RELEASES_REPO}/releases?per_page=10`, {
    headers: { Accept: 'application/vnd.github+json' },
    signal: AbortSignal.timeout(10_000),
  });
  if (!response.ok) throw new Error(`Release lookup failed: ${response.status}`);
  const releases = (await response.json()) as Release[];
  for (const release of releases) {
    if (release.draft) continue;
    const dmgs = release.assets.filter((asset) => asset.name.endsWith('.dmg'));
    const asset = dmgs.find((asset) => /arm64|aarch64|apple/i.test(asset.name)) ?? dmgs[0];
    if (asset) return { href: asset.browser_download_url, version: release.tag_name.replace(/^v/, '') };
  }
  throw new Error('No macOS release available');
}

/** Stargazer count, or null when GitHub is unreachable so the page still builds. */
export async function fetchStars(fetcher: typeof fetch): Promise<number | null> {
  try {
    const response = await fetcher(`https://api.github.com/repos/${RELEASES_REPO}`, {
      headers: { Accept: 'application/vnd.github+json' },
      signal: AbortSignal.timeout(10_000),
    });
    if (!response.ok) return null;
    const repo = (await response.json()) as { stargazers_count?: unknown };
    return typeof repo.stargazers_count === 'number' ? repo.stargazers_count : null;
  } catch {
    return null;
  }
}

/** 230 → "230", 1240 → "1.2k", 12400 → "12k". */
export function formatStars(count: number): string {
  if (count < 1000) return String(count);
  const k = count / 1000;
  return `${k < 10 ? k.toFixed(1).replace(/\.0$/, '') : Math.round(k)}k`;
}
