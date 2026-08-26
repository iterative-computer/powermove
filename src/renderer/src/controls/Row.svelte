<script module lang="ts">
  let sequence = 0;
</script>

<script lang="ts">
  import type { Snippet } from 'svelte';
  import { provideRowLabel } from './context';

  let {
    label,
    children,
    left,
    onLabel
  }: {
    label: string;
    children: Snippet;
    left?: Snippet;
    onLabel?: (event: PointerEvent) => void;
  } = $props();

  const labelId = `pm-control-label-${++sequence}`;
  provideRowLabel(labelId);
</script>

<div class="row split">
  {#if left}{@render left()}{/if}
  {#if onLabel}
    <button type="button" class="k" id={labelId} onpointerdown={onLabel}>{label}</button>
  {:else}
    <div class="k" id={labelId}>{label}</div>
  {/if}
  <div class="vwrap">{@render children()}</div>
</div>
