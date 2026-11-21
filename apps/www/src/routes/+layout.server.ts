import { latestDownloads, starCount } from '$lib/server/release';
import type { LayoutServerLoad } from './$types';

// Embed the release in the static HTML so hydration never adds the version late.
export const load: LayoutServerLoad = async ({ fetch }) => {
  const [downloads, stars] = await Promise.all([latestDownloads(fetch), starCount(fetch)]);
  return { ...downloads, stars };
};
