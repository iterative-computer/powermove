import InspectorPanel from './InspectorPanel.svelte';
import { showFxMenu } from './inspector/actions';
import {
  configureInspectorRegistry,
  inspectorRefresh
} from './inspector/refresh.svelte';
import { registerSveltePanel } from './registerSveltePanel';

type LegacyPM = Record<string, any>;

export function registerInspectorPanel(PM: LegacyPM): void {
  configureInspectorRegistry(PM);

  /* Keep the public runtime hook with the panel that owns uniform UI. */
  PM.syncShaderUniforms = (layer: any): void => {
    const definitions = PM.parseUniforms(layer.d.code);
    PM.UIState.setShaderMeta(layer, { udefs: definitions });
    const uniforms = layer.d.uniforms;
    for (const definition of definitions) {
      if (!uniforms[definition.name]) uniforms[definition.name] = PM.P(definition.def);
    }
    for (const name of Object.keys(uniforms)) {
      if (!definitions.some((definition: any) => definition.name === name)) delete uniforms[name];
    }
  };

  PM.Inspector = {
    refresh(): void {
      inspectorRefresh.bump();
    },
    body: null,
    syncs: [],
    focusText(layer: any): void {
      PM.selectLayers(layer.id);
      window.requestAnimationFrame(() => {
        const textarea = document.querySelector<HTMLTextAreaElement>(
          `textarea[data-inspector-text-layer="${CSS.escape(String(layer.id))}"]`
        );
        textarea?.focus();
        textarea?.select();
      });
    }
  };

  PM.fxMenu = (anchor: HTMLElement) => showFxMenu(PM, anchor);
  registerSveltePanel(PM, 'inspector', { title: 'Properties', component: InspectorPanel, header: () => {} });
}
