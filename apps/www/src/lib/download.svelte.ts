import { page } from '$app/state';
import { fetchDownload, type Download } from './releases';

let latest = $state<Download | undefined>();

/** Use the prerendered release immediately, then refresh for newly published builds. */
export const download = {
  get href() { return (latest ?? page.data.download).href; },
  get version() { return (latest ?? page.data.download).version; },
};

let started = false;

export async function resolveDownload() {
  if (started || typeof fetch === 'undefined') return;
  started = true;
  try {
    latest = await fetchDownload(fetch);
  } catch {
    /* Keep the fully populated release embedded in the page. */
  }
}
