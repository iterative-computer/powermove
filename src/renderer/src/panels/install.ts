/* Registry of Svelte panels that replace their legacy counterparts. Runs after
   the legacy ui/* installs (so PM.registerPanel exists) and before the app
   boots (so the workspace layout builds the Svelte version). */
import PerfPanel from './PerfPanel.svelte';
import { registerSveltePanel } from './registerSveltePanel';

type LegacyPM = Record<string, any>;

export function installSveltePanels(PM: LegacyPM): void {
  registerSveltePanel(PM, 'perf', { title: 'Performance', size: 150, component: PerfPanel });
}
