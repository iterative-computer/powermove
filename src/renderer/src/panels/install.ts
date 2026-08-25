/* Registry of Svelte panels that replace their legacy counterparts. Runs after
   the legacy ui/* installs (so PM.registerPanel exists) and before the app
   boots (so the workspace layout builds the Svelte version). */
import PerfPanel from './PerfPanel.svelte';
import { installGeneratedPanels } from './register-generated';
import { registerInspectorPanel } from './register-inspector';
import { registerShaderPanel } from './register-shader';
import { registerSimplePanels } from './register-simple';
import { registerSveltePanel } from './registerSveltePanel';

type LegacyPM = Record<string, any>;

export function installSveltePanels(PM: LegacyPM): void {
  registerSveltePanel(PM, 'perf', { title: 'Performance', size: 150, component: PerfPanel });
  registerSimplePanels(PM); // notes, takes, workspaces, assets, fxbrowser
  registerInspectorPanel(PM);
  registerShaderPanel(PM);
  installGeneratedPanels(PM); // patches PM.WS.registerCustom → GeneratedPanel
}
