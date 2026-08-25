import ShaderPanel from './ShaderPanel.svelte';
import { registerSveltePanel } from './registerSveltePanel';

type LegacyPM = Record<string, any>;

function focusShaderEditor(PM: LegacyPM): boolean {
  const body = PM.panelInst?.shader?.body as HTMLElement | undefined;
  const textarea = body?.querySelector<HTMLTextAreaElement>('textarea[data-shader-editor]')
    ?? document.querySelector<HTMLTextAreaElement>('#panel-shader textarea[data-shader-editor]');
  if (!textarea) return false;
  textarea.focus();
  return true;
}

export function registerShaderPanel(PM: LegacyPM): void {
  registerSveltePanel(PM, 'shader', {
    title: 'Shader',
    noscroll: true,
    size: 320,
    component: ShaderPanel
  });

  PM.openShaderEditor = (layer?: Record<string, any>): void => {
    if (layer) PM.selectLayers(layer.id);
    if (!PM.Layout.hasPanel(PM.WS.current, 'shader')) {
      PM.WS.mutate((workspace: Record<string, any>) => PM.Layout.addPanel(workspace, 'shader', 'center'));
    }

    PM.Layout.refresh?.('shader');
    if (!focusShaderEditor(PM)) {
      window.requestAnimationFrame(() => focusShaderEditor(PM));
    }
  };
}
