/*
 * Mods — the whole user-facing surface of the extension system.
 *
 * One list: what you added, then what shipped with the app. A switch each, a
 * way out of every failure. Everything else the kernel does is invisible here
 * on purpose; the panel never says "extension", it says "mod".
 */
import type { PowermoveAPI } from 'powermove';

import ModsPanel from './ModsPanel.svelte';

export default function activate(api: PowermoveAPI): void {
  api.panels.register({
    id: 'mods',
    title: 'Mods',
    size: 260,
    build(body) {
      // Mounted imperatively so the panel gets `api` — panel components are
      // handed only { panelId, spec } by the kernel.
      return api.host.mount(ModsPanel, body, { api });
    }
  });

  api.commands.register({
    id: 'mods.open',
    label: 'Show Mods',
    category: 'Mods',
    run: () => api.panels.open('mods', 'right')
  });
}
