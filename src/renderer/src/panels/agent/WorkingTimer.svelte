<script lang="ts">
  import { onMount } from 'svelte';
  let { startedAt }: { startedAt: number } = $props();
  let now = $state(Date.now());
  onMount(() => {
    const timer = setInterval(() => { now = Date.now(); }, 1000);
    return () => clearInterval(timer);
  });
  const seconds = $derived(Math.max(0, Math.floor((now - startedAt) / 1000)));
  const duration = $derived(seconds < 60 ? `${seconds}s`
    : seconds < 3600 ? `${Math.floor(seconds / 60)}m ${seconds % 60}s`
    : `${Math.floor(seconds / 3600)}h ${Math.floor(seconds % 3600 / 60)}m ${seconds % 60}s`);
</script>

<div class="agent-working-timer">Working for {duration}</div>

<style>
  .agent-working-timer{color:var(--tx-3);font-size:var(--fs-sm);line-height:1.5;font-variant-numeric:tabular-nums;padding:0 0 10px;margin:4px 0 12px;border-bottom:1px solid var(--line)}
</style>
