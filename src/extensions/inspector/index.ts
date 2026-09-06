import type { PowermoveAPI } from 'powermove';

import InspectorPanel from './InspectorPanel.svelte';
import { showFxMenu } from './actions';
import {
  clearEffectClipboard,
  copyEffects,
  effectClipboardSize,
  effectPasteCommands,
  type PasteEffectCommand
} from './effect-clipboard';
import { inspectorRefresh } from './refresh.svelte.js';

type LegacyPM = Record<string, any>;

export default function activate(api: PowermoveAPI): void {
  const PM = api.host.pm as LegacyPM;
  let activeEffectSelection: { layerId: string; ids: string[] } | null = null;

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
    },
    setEffectSelection(layerId: string, ids: string[]): void {
      activeEffectSelection = ids.length ? { layerId, ids: [...ids] } : null;
    },
    clearEffectSelection(): void {
      activeEffectSelection = null;
    },
    copySelectedEffects(): boolean {
      if (!activeEffectSelection) return false;
      const layer = PM.L?.(activeEffectSelection.layerId);
      const wanted = new Set(activeEffectSelection.ids);
      const effects = (layer?.fx ?? []).filter((effect: any) => wanted.has(effect.id));
      if (!effects.length) {
        activeEffectSelection = null;
        return false;
      }
      const count = copyEffects(effects);
      PM.toast?.(`Copied ${count} ${count === 1 ? 'effect' : 'effects'}`);
      return true;
    },
    clearEffectClipboard,
    pasteCopiedEffects(): boolean {
      if (!effectClipboardSize()) return false;
      const targets = PM.selLayers?.().filter((layer: any) => layer?.type !== 'audio') ?? [];
      if (!targets.length) {
        PM.toast?.('Select a layer to paste the effect');
        return true;
      }
      const commands: PasteEffectCommand[] = targets.flatMap((layer: any) => effectPasteCommands(layer.id))
        .filter((command: PasteEffectCommand) => !!PM.FX?.[command.effect]);
      if (!commands.length) {
        PM.toast?.('The copied effect is not available');
        return true;
      }
      const count = effectClipboardSize();
      const result = PM.Edit.apply(commands.length === 1 ? commands[0] : commands, {
        label: commands.length === 1 ? 'Paste effect' : 'Paste effects',
        origin: 'inspector'
      });
      if (result?.ok === false) return true;
      PM.invalidate?.();
      const pasted = count * targets.length;
      PM.toast?.(`Pasted ${pasted} ${pasted === 1 ? 'effect' : 'effects'}`);
      return true;
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
    icon: 'sliders',
    title: 'Properties',
    component: InspectorPanel as any,
    header: () => {}
  });
}
