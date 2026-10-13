import { dev } from '$app/environment';
import { RELEASES } from '$lib/links';
import { fetchDownload, fetchStars } from '$lib/releases';
import type { LayoutServerLoad } from './$types';

// Embed the release in the static HTML so hydration never adds the version late.
// Fail the build if it cannot be resolved, leaving the previous deployment intact.
// In dev, a rate-limited GitHub falls back to the releases page so the site still renders.
// The star count is decorative: a failed lookup leaves it out rather than failing the build.
export const load: LayoutServerLoad = async ({ fetch }) => {
  const [download, stars] = await Promise.all([
    fetchDownload(fetch).catch((error) => {
      if (!dev) throw error;
      return { href: RELEASES, version: '' };
    }),
    fetchStars(fetch),
  ]);
  return { download, stars };
};
