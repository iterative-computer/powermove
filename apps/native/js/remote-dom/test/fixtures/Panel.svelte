<script>
  let expanded = $state(false);
  let key = $state('');
</script>
<svelte:window onkeydown={event => key = event.key} />
<button onclick={() => expanded = !expanded}>Toggle</button>
{#if expanded}
  <svg viewBox="0 0 10 10"><path d="M1 2L3 4" style:stroke="red" /></svg>
  <p>{key || 'Ready'}</p>
{:else}
  <p>Closed</p>
{/if}
