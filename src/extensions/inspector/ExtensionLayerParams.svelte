<script lang="ts">
  import { inspectorContext, type EditBinding } from './context';
  import AnimatedRow from './AnimatedRow.svelte';
  import ChannelRow from './ChannelRow.svelte';

  const { api, doc, transport } = inspectorContext();
  const { ColorField, Section, ToggleField } = api.ui.controls;

  let { PM, layer }: { PM: Record<string, any>; layer: Record<string, any> } = $props();
  const definition = $derived((doc.tick.structure, doc.proj, api.layers.get(String(layer.d?.definition || ''))));
  const meshAsset = $derived.by(() => {
    doc.tick.assets; doc.proj;
    if (definition?.renderer.kind !== 'mesh') return null;
    return api.assets.get(String(layer.d?.data?.[definition.renderer.assetField] || '')) ?? null;
  });

  function fieldBinding(path: string, label: string): EditBinding {
    return {
      mode: 'command',
      label,
      origin: 'inspector',
      command: (value: unknown) => ({
        type: 'set_property', target: layer.id, path, value: value as any,
        time: PM.time, preserveHandEdits: false
      })
    };
  }
</script>

{#if !definition}
  <Section title="Extension" />
  <div class="extension-layer-missing" role="status">
    <span>Renderer unavailable</span>
    <code>{String(layer.d?.definition || 'Unknown definition')}</code>
    <small>The structured layer data is preserved.</small>
  </div>
{:else}
  <Section title={definition.label} />
  {#if Number(layer.d?.version) !== definition.version}
    <div class="extension-layer-warning" role="status">
      Saved with definition v{Number(layer.d?.version) || 1}; installed definition is v{definition.version}.
    </div>
  {/if}
  {#if definition.renderer.kind === 'mesh'}
    <div class:missing={!meshAsset} class="extension-layer-asset" role="status">
      <span>{meshAsset ? 'Model asset' : 'Model asset missing'}</span>
      <code>{meshAsset?.name || String(layer.d?.data?.[definition.renderer.assetField] || 'No OBJ selected')}</code>
      {#if meshAsset?.triangles}<small>{Number(meshAsset.triangles).toLocaleString()} triangles</small>{/if}
    </div>
  {/if}
  {#each definition.params as parameter (parameter.k)}
    {@const property = layer.d?.params?.[parameter.k]}
    {#if property}
      {@const path = `x.${parameter.k}`}
      {#if parameter.type === 'color'}
        <AnimatedRow {PM} {layer} {path} label={parameter.label}>
          <ColorField
            {PM}
            get={() => (doc.tick.values, doc.proj, transport.time, PM.evP(layer, property, transport.time, path))}
            edit={fieldBinding(path, parameter.label)}
            label={parameter.label}
          />
        </AnimatedRow>
      {:else if parameter.type === 'toggle'}
        <AnimatedRow {PM} {layer} {path} label={parameter.label}>
          <ToggleField
            {PM}
            get={() => (doc.tick.values, doc.proj, transport.time, PM.evP(layer, property, transport.time, path))}
            edit={fieldBinding(path, parameter.label)}
            label={parameter.label}
          />
        </AnimatedRow>
      {:else}
        <ChannelRow
          {PM}
          {layer}
          channel={path}
          label={parameter.label}
          {property}
          getValue={(time) => PM.evP(layer, property, time, parameter.k)}
          step={parameter.step ?? ((parameter.max - parameter.min) / 200 || 0.01)}
          min={parameter.min}
          max={parameter.max}
          unit={parameter.unit}
          precision={3}
        />
      {/if}
    {/if}
  {/each}
{/if}

<style>
  .extension-layer-missing,
  .extension-layer-warning,
  .extension-layer-asset {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin: 4px 0 8px;
    padding: 9px 10px;
    border-radius: var(--r-sm);
    background: var(--bg-row);
    color: var(--tx-2);
    font-size: var(--fs-xs);
    line-height: 1.4;
  }
  .extension-layer-missing code { color: var(--tx); font-family: var(--font-mono); }
  .extension-layer-missing small { color: var(--tx-3); }
  .extension-layer-warning { color: var(--orange, #d8864a); }
  .extension-layer-asset code { overflow: hidden; color: var(--tx); font-family: var(--font-mono); text-overflow: ellipsis; white-space: nowrap; }
  .extension-layer-asset small { color: var(--tx-3); }
  .extension-layer-asset.missing { color: var(--orange, #d8864a); }
</style>
