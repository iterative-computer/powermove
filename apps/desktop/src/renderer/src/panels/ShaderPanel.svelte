<script lang="ts">
  import { fancySelect } from '../controls/select/enhance';
  import type { ShaderLayer } from '../core/types/project';
  import { EditGesture, type EditBinding } from '../controls/gesture';
  import { doc } from '../state/document.svelte';
  import { sel } from '../state/selection.svelte';
  import type { PanelProps } from './registerSveltePanel';
  import CompileStatus from './shader/CompileStatus.svelte';
  import ShaderEditor from './shader/ShaderEditor.svelte';

  let { panelId }: PanelProps = $props();

  const PM = window.PM as Record<string, any>;
  const selectedShader = $derived.by(() => {
    doc.tick.structure;
    doc.tick.values;
    const selectedId = sel.layers[0];
    const layers = doc.proj?.layers ?? [];
    const layer = layers.find((candidate) => candidate.id === selectedId);
    return layer?.type === 'shader' ? layer : layers.find((candidate) => candidate.type === 'shader') ?? null;
  });
  const presets = Object.entries(PM.SHADER_PRESETS ?? {}) as Array<[string, string]>;
  const presetId = $derived(`shader-preset-${panelId}`);
  let refreshToken = $state(0);

  function binding(layer: ShaderLayer, label: string): EditBinding {
    return {
      mode: 'command',
      label,
      origin: 'shader-panel',
      command: (value) => ({ type: 'set_content', target: layer.id, patch: { code: String(value ?? '') } })
    };
  }

  function applyPreset(event: Event): void {
    event.stopPropagation();
    if (!selectedShader) return;
    const select = event.currentTarget as HTMLSelectElement;
    const preset = presets.find(([name]) => name === select.value);
    select.value = '';
    if (!preset) return;
    new EditGesture(PM, binding(selectedShader, 'Shader preset')).once(preset[1]);
    PM.invalidate?.();
    refreshToken++;
  }

  function compile(): void {
    if (!selectedShader) return;
    const meta = PM.UIState.getShaderMeta(selectedShader);
    if (meta.shaderKey != null) PM.GL.dropProgram(meta.shaderKey);
    PM.invalidate?.();
    PM.Inspector?.refresh?.();
    refreshToken++;
  }
</script>

<div class="shader-panel" data-svelte-panel={panelId}>
  {#if selectedShader}
    {#key selectedShader.id}
      <ShaderEditor PM={PM} layer={selectedShader} {panelId} onWrite={() => refreshToken++} onCompile={compile} />
      <div class="codebar">
        <CompileStatus PM={PM} layer={selectedShader} {refreshToken} />
        <span class="codebar-spacer"></span>
        <label class="panel-sr-only" for={presetId}>Shader preset</label>
        <select
          id={presetId}
          class="sel preset-select"
          use:fancySelect
          value=""
          onchange={applyPreset}
          onpointerdown={(event) => event.stopPropagation()}
          onkeydown={(event) => event.stopPropagation()}
        >
          <option value="">Presets…</option>
          {#each presets as [name] (name)}
            <option value={name}>{name}</option>
          {/each}
        </select>
        <button class="chip compile-button" type="button" onclick={compile}>Compile ⌘↵</button>
      </div>
    {/key}
  {:else}
    <div class="empty shader-empty">
      <b>No shader layers</b>
      <span>Create a shader layer (⌘⇧G) to edit its source.</span>
    </div>
  {/if}
</div>

<style>
  .shader-panel {
    display: flex;
    flex-direction: column;
    width: 100%;
    height: 100%;
    min-height: 0;
  }

  .shader-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 5px;
    height: 100%;
    padding: 18px;
    color: var(--tx-4);
    text-align: center;
  }

  .shader-empty b {
    color: var(--tx-3);
    font-size: var(--fs-sm);
    font-weight: 560;
  }

  .shader-empty span {
    max-width: 220px;
    font-size: var(--fs-xs);
    line-height: 1.45;
  }

  .codebar-spacer {
    flex: 1;
  }

  .preset-select {
    max-width: 112px;
  }

  .compile-button {
    height: 24px;
  }
</style>
