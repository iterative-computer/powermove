import type { InspectorService, Layer, PowermoveAPI } from 'powermove';

import InspectorPanel from './InspectorPanel.svelte';
import {
  clearEffectClipboard,
  copyEffects,
  effectClipboardSize,
  effectPasteCommands
} from './effect-clipboard';
import type { InspectorRuntimeService } from './context';
import { inspectorRefresh } from './refresh.svelte.js';
import { shaderDefinitions, syncShaderUniforms } from './shader';

export default function activate(api: PowermoveAPI): void {
  let activeEffectSelection: { layerId: string; ids: string[] } | null = null;

  const inspector: InspectorRuntimeService = {
    refresh(): void {
      inspectorRefresh.bump();
    },
    body: null,
    syncs: [],
    focusText(layer: Layer): void {
      api.selection.select([layer.id]);
      window.requestAnimationFrame(() => {
        const textarea = document.querySelector<HTMLTextAreaElement>(
          `textarea[data-inspector-text-layer="${CSS.escape(String(layer.id))}"]`
        );
        textarea?.focus();
        textarea?.select();
      });
    },
    setEffectSelection(layerId: string, ids: string[]): void {
      activeEffectSelection = ids.length ? { layerId, ids: [...ids] } : null;
    },
    clearEffectSelection(): void {
      activeEffectSelection = null;
    },
    copySelectedEffects(): boolean {
      if (!activeEffectSelection) return false;
      const layer = api.model.layer(activeEffectSelection.layerId);
      const wanted = new Set(activeEffectSelection.ids);
      const effects = (layer?.fx ?? []).filter((effect) => wanted.has(effect.id));
      if (!effects.length) {
        activeEffectSelection = null;
        return false;
      }
      const count = copyEffects(effects);
      api.ui.toast(`Copied ${count} ${count === 1 ? 'effect' : 'effects'}`);
      return true;
    },
    clearEffectClipboard,
    pasteCopiedEffects(): boolean {
      if (!effectClipboardSize()) return false;
      const targets = api.selection.layers()
        .map((id) => api.model.layer(id))
        .filter((layer): layer is Layer => layer !== null && layer.type !== 'audio');
      if (!targets.length) {
        api.ui.toast('Select a layer to paste the effect');
        return true;
      }
      const commands = targets.flatMap((layer) => effectPasteCommands(layer.id))
        .filter((command) => !!api.effects.get(command.effect));
      if (!commands.length) {
        api.ui.toast('The copied effect is not available');
        return true;
      }
      const count = effectClipboardSize();
      const result = api.edit.apply(commands.length === 1 ? commands[0]! : commands, {
        label: commands.length === 1 ? 'Paste effect' : 'Paste effects',
        origin: 'inspector'
      });
      if (result?.ok === false) return true;
      api.transport.invalidate();
      const pasted = count * targets.length;
      api.ui.toast(`Pasted ${pasted} ${pasted === 1 ? 'effect' : 'effects'}`);
      return true;
    },
    syncShaderUniforms: (layer) => syncShaderUniforms(api, layer),
    shaderDefinitions
  };
  api.services.register<InspectorService>('inspector', inspector);
  api.services.register('shaderHooks', { syncShaderUniforms: inspector.syncShaderUniforms });

  /* The app hydrates before built-ins activate. Bring every already-loaded
     shader onto the inspector-owned uniform contract immediately. */
  const project = api.project.get();
  const containers = [project, ...Object.values(project.comps ?? {})];
  for (const container of containers) {
    for (const layer of container?.layers ?? []) {
      if (layer.type !== 'shader') continue;
      try {
        inspector.syncShaderUniforms(layer);
      } catch (error) {
        api.log('warn', `could not synchronize shader uniforms for ${String(layer.id ?? 'unknown')}`, error);
      }
    }
  }

  api.panels.register({
    id: 'inspector',
    icon: 'sliders',
    title: 'Properties',
    component: InspectorPanel as any,
    header: () => {}
  });
}
