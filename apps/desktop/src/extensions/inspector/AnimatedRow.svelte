<script lang="ts">
  import type { Snippet } from 'svelte';
  import { inspectorContext } from './context';
  import PropertyStopwatch from './PropertyStopwatch.svelte';
  const { api } = inspectorContext();
  const { Row } = api.ui.controls;
  let { layer, path, paths, label, children }: {
    layer: any; path?: string; paths?: string[]; label: string; children: Snippet;
  } = $props();
</script>
<Row {api} {label}>
  {#snippet left()}
    {#if path}<PropertyStopwatch {layer} {path} {paths} {label} fallback={path.startsWith('c.') ? layer.d?.[path.slice(2)] : undefined} />{/if}
  {/snippet}
  {@render children()}
</Row>
