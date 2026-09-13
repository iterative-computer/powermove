<script lang="ts">
  import type { Snippet } from 'svelte';
  import { inspectorContext } from './context';
  import PropertyStopwatch from './PropertyStopwatch.svelte';
  const { api } = inspectorContext();
  const { Row } = api.ui.controls;
  let { layer, path, label, children }: {
    layer: any; path?: string; label: string; children: Snippet;
  } = $props();
</script>
<Row {label}>
  {#snippet left()}
    {#if path}<PropertyStopwatch {layer} {path} {label} fallback={path.startsWith('c.') ? layer.d?.[path.slice(2)] : undefined} />{/if}
  {/snippet}
  {@render children()}
</Row>
