<script lang="ts">
  import { onMount } from 'svelte';
  import { workDuration } from './work-groups';
  let { startedAt }: { startedAt: number } = $props();
  let now = $state(Date.now());
  onMount(() => {
    const timer = setInterval(() => { now = Date.now(); }, 1000);
    return () => clearInterval(timer);
  });
  const duration = $derived(workDuration(now - startedAt));
</script>

<div class="agent-working-timer">Working for {duration}</div>

<style>
  .agent-working-timer{color:var(--tx-3);font-size:var(--fs-sm);line-height:1.5;font-variant-numeric:tabular-nums;padding:0 0 10px;margin:4px 0 0;border-bottom:1px solid var(--line)}
</style>
