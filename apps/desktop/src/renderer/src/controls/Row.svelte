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
    onLabel,
    pair = false,
    action
  }: {
    label: string;
    children: Snippet;
    left?: Snippet;
    onLabel?: (event: PointerEvent) => void;
    /** Two controls side by side (X / Y, W / H). */
    pair?: boolean;
    /** A 24px slot after a pair (link, lock, …). */
    action?: Snippet;
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
  <div class="vwrap" class:pair>
    {#if pair}
      <div class="pair-group">{@render children()}</div>
      {#if action}{@render action()}{/if}
    {:else}
      {@render children()}
    {/if}
  </div>
</div>
