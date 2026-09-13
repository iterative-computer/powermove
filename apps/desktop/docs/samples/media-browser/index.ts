import type { PowermoveAPI } from 'powermove';

import MediaBrowserPanel from './MediaBrowserPanel.svelte';

export interface MediaItem { id: string; name: string; url: string }
export type MediaSource = () => Promise<readonly MediaItem[]>;

// Data URLs keep the sample deterministic and avoid network/CSP surprises.
// Replace this function with a fetch to your own catalogue; the panel contract
// stays unchanged.
export const mockMediaSource: MediaSource = async () => [
  {
    id: 'blue-square',
    name: 'Blue square.svg',
    url: 'data:image/svg+xml;charset=utf-8,%3Csvg xmlns="http://www.w3.org/2000/svg" width="320" height="320"%3E%3Crect width="320" height="320" fill="%236a8dff"/%3E%3C/svg%3E'
  },
  {
    id: 'gold-circle',
    name: 'Gold circle.svg',
    url: 'data:image/svg+xml;charset=utf-8,%3Csvg xmlns="http://www.w3.org/2000/svg" width="320" height="320"%3E%3Ccircle cx="160" cy="160" r="132" fill="%23d2a84a"/%3E%3C/svg%3E'
  }
];

export default function activate(api: PowermoveAPI): void {
  api.panels.register({
    id: 'media-browser',
    title: 'Media Browser',
    size: 240,
    build(body) {
      // `build` lets a real extension inject any MediaSource while keeping the
      // Svelte panel concerned only with presentation and typed API calls.
      return api.host.mount(MediaBrowserPanel, body, { api, source: mockMediaSource });
    }
  });

  api.commands.register({
    id: 'media-browser.open',
    label: 'Show Media Browser',
    category: 'Panels',
    run: () => api.panels.open('media-browser', { dock: 'left', index: 0 })
  });

  // Indexed placement is deterministic even when the workspace already has
  // other left-docked panels.
  api.panels.open('media-browser', { dock: 'left', index: 0 });
}
