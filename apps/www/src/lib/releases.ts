import { RELEASES_REPO } from './links';

type Asset = { name: string; browser_download_url: string };
type Release = { draft: boolean; prerelease?: boolean; tag_name: string; assets: Asset[] };
export type Download = { href: string; version: string };
export type Downloads = { download: Download; previousDownload?: Download };

export async function fetchDownloads(fetcher: typeof fetch): Promise<Downloads> {
  const response = await fetcher(`https://api.github.com/repos/${RELEASES_REPO}/releases?per_page=100`, {
    headers: { Accept: 'application/vnd.github+json' },
    signal: AbortSignal.timeout(10_000),
  });
  if (!response.ok) throw new Error(`Release lookup failed: ${response.status}`);
  const releases = (await response.json()) as Release[];
  let native: Download | undefined;
  let electron: Download | undefined;
  for (const release of releases) {
    if (release.draft || release.prerelease) continue;

    const nativeVersion = /^native-v(\d+\.\d+\.\d+)$/.exec(release.tag_name)?.[1];
    if (nativeVersion && !native) {
      const asset = release.assets.find(({ name }) => name === `Powermove-Native-${nativeVersion}-mac-arm64.dmg`);
      if (asset) native = { href: asset.browser_download_url, version: nativeVersion };
    }

    const electronVersion = /^v?(\d+(?:\.\d+){0,2})$/.exec(release.tag_name)?.[1];
    if (electronVersion && !electron) {
      const dmgs = release.assets.filter(({ name }) => name.endsWith('.dmg'));
      const asset = dmgs.find(({ name }) => /arm64|aarch64|apple/i.test(name)) ?? dmgs[0];
      if (asset) electron = { href: asset.browser_download_url, version: electronVersion };
    }

    if (native && electron) break;
  }
  if (!electron) throw new Error('No macOS release available for the previous version');
  return native ? { download: native, previousDownload: electron } : { download: electron };
}

export async function fetchDownload(fetcher: typeof fetch): Promise<Download> {
  return (await fetchDownloads(fetcher)).download;
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
