import { fetchDownload } from '$lib/releases';
import type { LayoutServerLoad } from './$types';

// Embed the release in the static HTML so hydration never adds the version late.
// Fail the build if it cannot be resolved, leaving the previous deployment intact.
export const load: LayoutServerLoad = async ({ fetch }) => ({
  download: await fetchDownload(fetch),
});
