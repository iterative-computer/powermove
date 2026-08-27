<script lang="ts">
  import Icon from './Icon.svelte';
  import { inspectorContext } from './context';
  import { showFxMenu, showNewLayerMenu } from './actions';

  const { doc } = inspectorContext();

  let {
    PM,
    layer
  }: {
    PM: Record<string, any>;
    layer?: any;
  } = $props();

  const layerName = $derived((doc.tick.structure, doc.proj, layer?.name ?? 'Properties'));
  const typeLabel = $derived((doc.tick.structure, doc.proj, layer ? PM.TYPE_META?.[layer.type]?.label : ''));
</script>

<!-- registerSveltePanel has no Svelte header lifecycle, so the legacy header
     content lives at the top of the body until DockLayout owns panel chrome. -->
<div class="row inspector-header" data-inspector-header>
  <div class="k">
    <span>{layerName}</span>
    {#if typeLabel}<span class="sub"> · {typeLabel}</span>{/if}
  </div>
  <button
    type="button"
    class="iconbtn"
    title="New layer"
    aria-label="New layer"
    onpointerdown={(event) => {
      event.preventDefault();
      showNewLayerMenu(PM, event.currentTarget);
    }}
  ><Icon name="layers" /></button>
  {#if layer?.type !== 'audio'}
    <button
      type="button"
      class="iconbtn"
      title="Add effect"
      aria-label="Add effect"
      onpointerdown={(event) => {
        event.preventDefault();
        showFxMenu(PM, event.currentTarget, layer);
      }}
    ><Icon name="plus" /></button>
  {/if}
</div>
