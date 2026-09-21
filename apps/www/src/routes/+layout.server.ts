import { fetchDownload, fetchStars } from '$lib/releases';
import type { LayoutServerLoad } from './$types';

// Embed the release in the static HTML so hydration never adds the version late.
// Fail the build if it cannot be resolved, leaving the previous deployment intact.
// The star count is decorative: a failed lookup leaves it out rather than failing the build.
export const load: LayoutServerLoad = async ({ fetch }) => {
  const [download, stars] = await Promise.all([fetchDownload(fetch), fetchStars(fetch)]);
  return { download, stars };
};
