<script lang="ts">
  import type { Snippet } from 'svelte';
  import type { PowermoveAPI } from '../kernel/api';

  let { title, children, api }: { title?: string; children?: Snippet; api: PowermoveAPI } = $props();
</script>

<div class="sec">{title ?? ''}{#if children}{@render children()}{/if}</div>
