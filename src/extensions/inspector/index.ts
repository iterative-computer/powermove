import type { PowermoveAPI } from 'powermove';

import InspectorPanel from './InspectorPanel.svelte';
import { showFxMenu } from './actions';
import { inspectorRefresh } from './refresh.svelte.js';

type LegacyPM = Record<string, any>;

export default function activate(api: PowermoveAPI): void {
  const PM = api.host.pm as LegacyPM;

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

  /* The app hydrates before built-ins activate. Bring every already-loaded
     shader onto the inspector-owned uniform contract immediately. */
  const containers = [PM.proj, ...Object.values(PM.proj?.comps ?? {})] as any[];
  for (const container of containers) {
    for (const layer of container?.layers ?? []) {
      if (layer?.type !== 'shader') continue;
      try {
        PM.syncShaderUniforms(layer);
      } catch (error) {
        api.log('warn', `could not synchronize shader uniforms for ${String(layer.id ?? 'unknown')}`, error);
      }
    }
  }

  api.panels.register({
    id: 'inspector',
    title: 'Properties',
    component: InspectorPanel as any,
    header: () => {}
  });
}
