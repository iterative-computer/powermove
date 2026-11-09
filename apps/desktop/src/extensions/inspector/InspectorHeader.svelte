<script lang="ts">
  import { inspectorContext } from './context';

  const { api, doc } = inspectorContext();

  let { layer }: { layer?: any } = $props();

  const layerName = $derived((doc.tick.structure, doc.proj, layer?.name ?? 'Properties'));
  const typeLabel = $derived((doc.tick.structure, doc.proj, layer
    ? api.model.layerKind?.(layer)?.label
      || (layer.type === 'extension' ? api.model.layerDefinition(String(layer.d?.definition ?? ''))?.label : null)
      || (api.model.TYPE_META as Record<string, { label?: string }>)[layer.type]?.label
    : ''));
</script>

<!-- registerSveltePanel has no Svelte header lifecycle, so the legacy header
     content lives at the top of the body until DockLayout owns panel chrome. -->
<div class="row inspector-header" data-inspector-header>
  <div class="k">
    <span>{layerName}</span>
    <!-- "Terrain · Terrain" says nothing twice: the kind shows only when the name differs. -->
    {#if typeLabel && String(typeLabel).toLowerCase() !== String(layerName).trim().toLowerCase()}<span class="sub"> · {typeLabel}</span>{/if}
  </div>
</div>
