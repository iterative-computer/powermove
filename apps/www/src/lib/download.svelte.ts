import { page } from '$app/state';
import { fetchDownload, fetchStars, formatStars, type Download } from './releases';

let latest = $state<Download | undefined>();
let latestStars = $state<number | null | undefined>();

/** Use the prerendered release immediately, then refresh for newly published builds. */
export const download = {
  get href() { return (latest ?? page.data.download).href; },
  get version() { return (latest ?? page.data.download).version; },
};

/** Prerendered GitHub star count, refreshed on the client. Empty string when unknown. */
export const stars = {
  get label() {
    const count = latestStars ?? (page.data.stars as number | null);
    return typeof count === 'number' ? formatStars(count) : '';
  },
};

let started = false;

export async function resolveDownload() {
  if (started || typeof fetch === 'undefined') return;
  started = true;
  void fetchStars(fetch).then((count) => { if (count !== null) latestStars = count; });
  try {
    latest = await fetchDownload(fetch);
  } catch {
    /* Keep the fully populated release embedded in the page. */
  }
}
