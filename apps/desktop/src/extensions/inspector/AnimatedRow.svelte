<script lang="ts">
  import type { Snippet } from 'svelte';
  import { inspectorContext } from './context';
  import PropertyStopwatch from './PropertyStopwatch.svelte';
  const { Row } = inspectorContext().api.ui.controls;
  let { PM, layer, path, label, children }: {
    PM: Record<string, any>; layer: any; path?: string; label: string; children: Snippet;
  } = $props();
</script>
<Row {label}>
  {#snippet left()}
    {#if path}<PropertyStopwatch {PM} {layer} {path} {label} fallback={path.startsWith('c.') ? layer.d?.[path.slice(2)] : undefined} />{/if}
  {/snippet}
  {@render children()}
</Row>
