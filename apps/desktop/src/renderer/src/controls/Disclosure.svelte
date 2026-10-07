<script module lang="ts">
  const remembered = new Map<string, boolean>();
</script>
<script lang="ts">
  import { untrack, type Snippet } from 'svelte';
  let { children, label = 'More', count, changed = false, remember }: { children: Snippet; label?: string; count?: number; changed?: boolean; remember?: string } = $props();
  let open = $state(untrack(()=>remember ? remembered.get(remember) ?? false : false));
</script>
<details class="control-disclosure" {open} ontoggle={(event)=>{open=event.currentTarget.open;if(remember){if(remembered.size>1000)remembered.clear();remembered.set(remember,open);}}}>
  <summary>{label}{#if count !== undefined}<span class="disclosure-count">{count}</span>{/if}{#if changed}<span class="disclosure-changed" role="img" aria-label="Contains changed or animated settings" title="Changed or animated settings"></span>{/if}</summary>
  {@render children()}
</details>
<style>
  .control-disclosure { margin-top:4px; min-width:0; }
  summary { cursor:pointer; color:var(--tx-2); font-size:var(--fs-sm); padding:6px 4px; user-select:none; }
  summary:hover { color:var(--tx); }
  summary:focus-visible { outline:2px solid var(--accent); outline-offset:1px; border-radius:var(--r-sm); }
  .disclosure-count { margin-left:6px; color:var(--tx-3); font-variant-numeric:tabular-nums; }
  .disclosure-changed { display:inline-block; width:5px; height:5px; border-radius:50%; background:var(--accent); margin-left:7px; vertical-align:middle; }
</style>
