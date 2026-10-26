<script lang="ts">
  import { inspectorContext } from './context';

  const { api, doc } = inspectorContext();

  let { layer }: { layer?: any } = $props();

  const layerName = $derived((doc.tick.structure, doc.proj, layer?.name ?? 'Properties'));
  const typeLabel = $derived((doc.tick.structure, doc.proj, layer
    ? (layer.type === 'extension' ? api.model.layerDefinition(String(layer.d?.definition ?? ''))?.label : null)
      || (api.model.TYPE_META as Record<string, { label?: string }>)[layer.type]?.label
    : ''));
</script>

<!-- registerSveltePanel has no Svelte header lifecycle, so the legacy header
     content lives at the top of the body until DockLayout owns panel chrome. -->
<div class="row inspector-header" data-inspector-header>
  <div class="k">
    <span>{layerName}</span>
    {#if typeLabel}<span class="sub"> · {typeLabel}</span>{/if}
  </div>
</div>
