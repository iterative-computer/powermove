/* Unconditional registry of the Svelte-owned panels. Runs after the panel
   registry bootstrap (so PM.registerPanel exists) and before the app boots
   and applies the workspace layout. */
import PerfPanel from './PerfPanel.svelte';
import { installGeneratedPanels } from './register-generated';
import { registerShaderPanel } from './register-shader';
import { registerSimplePanels } from './register-simple';
import { registerSveltePanel } from './registerSveltePanel';

type LegacyPM = Record<string, any>;

export function installSveltePanels(PM: LegacyPM): void {
  registerSveltePanel(PM, 'perf', { title: 'Performance', size: 150, component: PerfPanel });
  registerSimplePanels(PM); // notes, takes, workspaces, assets, fxbrowser
  registerShaderPanel(PM);
  installGeneratedPanels(PM); // patches PM.WS.registerCustom → GeneratedPanel
  // The agent panel self-registers from legacy/assistant/spatial.ts (it owns the bridge).
}
