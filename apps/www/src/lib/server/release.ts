import { building, dev } from '$app/environment';
import { RELEASES } from '$lib/links';
import { fetchDownloads, fetchStars, type Download, type Downloads } from '$lib/releases';

/* Every prerendered page and text endpoint needs the latest release. During a
   build, look it up once and share the answer, so adding pages doesn't spend
   GitHub's unauthenticated rate limit. Outside a build, ask every time. */

let downloads: Promise<Downloads> | undefined;
let stars: Promise<number | null> | undefined;

// Fail the build if the release cannot be resolved, leaving the previous deployment intact.
// In dev, a rate-limited GitHub falls back to the releases page so the site still renders.
export function latestDownloads(fetcher: typeof fetch): Promise<Downloads> {
  if (building && downloads) return downloads;
  const lookup = fetchDownloads(fetcher).catch((error) => {
    if (!dev) throw error;
    return { download: { href: RELEASES, version: '' } };
  });
  if (building) downloads = lookup;
  return lookup;
}

export async function latestDownload(fetcher: typeof fetch): Promise<Download> {
  return (await latestDownloads(fetcher)).download;
}

/** The star count is decorative: a failed lookup leaves it out rather than failing the build. */
export function starCount(fetcher: typeof fetch): Promise<number | null> {
  if (building && stars) return stars;
  const lookup = fetchStars(fetcher);
  if (building) stars = lookup;
  return lookup;
}
