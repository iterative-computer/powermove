<script lang="ts">
  import type { Snippet } from 'svelte';

  let { title, children }: { title?: string; children?: Snippet } = $props();
</script>

<div class="sec">{title ?? ''}{#if children}{@render children()}{/if}</div>

