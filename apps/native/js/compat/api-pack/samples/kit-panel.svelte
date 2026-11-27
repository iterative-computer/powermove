<script lang="ts">
  let query = $state('');
  let format = $state('all');
  let selected = $state<string | null>(null);
  let { items = [] }: { items?: { id: string; name: string; preview: string; kind: string }[] } = $props();
  const formats = ['all', 'svg', 'png'];
  let shown = $derived(items.filter(it => it.name.toLowerCase().includes(query.toLowerCase()) && (format === 'all' || it.kind === format)));
</script>
<div class="pm-toolbar">
  <input class="pm-search" type="search" aria-label="Search items" placeholder="Search items" bind:value={query} />
</div>
<div class="pm-segmented" role="group" aria-label="Format">
  {#each formats as f}<button aria-pressed={format === f} onclick={() => format = f}>{f}</button>{/each}
</div>
{#if shown.length}
  <div class="pm-grid">
    {#each shown as it (it.id)}
      <button class="pm-card" aria-pressed={selected === it.id} onclick={() => selected = it.id}>
        <div class="pm-card-media"><img src={it.preview} alt="" /></div>
        <span class="pm-card-title">{it.name}</span><span class="pm-card-meta">{it.kind}</span>
      </button>
    {/each}
  </div>
{:else}<div class="pm-empty"><h2>No items found</h2><button class="pm-button" onclick={() => { query = ''; format = 'all'; }}>Clear filters</button></div>{/if}
<style>.pm-grid{padding:var(--pm-inset)}</style>
