<script lang="ts">
  import { inspectorContext } from './context';
  import ParameterList from './ParameterList.svelte';

  const { api, doc } = inspectorContext();
  const { Section } = api.ui.controls;

  let { layer }: { layer: any } = $props();
  const definition = $derived((doc.tick.structure, doc.proj, api.layers.get(String(layer.d?.definition || ''))));
  const meshAsset = $derived.by(() => {
    doc.tick.assets; doc.proj;
    if (definition?.renderer.kind !== 'mesh') return null;
    return api.assets.get(String(layer.d?.data?.[definition.renderer.assetField] || '')) ?? null;
  });


</script>

{#if !definition}
  <Section {api} title="Extension" />
  <div class="extension-layer-missing" role="status">
    <span>Renderer unavailable</span>
    <code>{String(layer.d?.definition || 'Unknown definition')}</code>
    <small>The structured layer data is preserved.</small>
  </div>
{:else if definition.renderer.kind === 'scene3d' || definition.renderer.kind === 'layer3d'}
  <!-- Scene layers are edited through the 3D Layers extension's Scene section. -->
{:else}
  <Section {api} title={definition.label} />
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
  <ParameterList {layer} params={definition.params} properties={layer.d?.params ?? {}} prefix="x" ui={definition.ui} />
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
