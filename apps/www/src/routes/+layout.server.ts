import { latestDownload, starCount } from '$lib/server/release';
import type { LayoutServerLoad } from './$types';

// Embed the release in the static HTML so hydration never adds the version late.
export const load: LayoutServerLoad = async ({ fetch }) => {
  const [download, stars] = await Promise.all([latestDownload(fetch), starCount(fetch)]);
  return { download, stars };
};
