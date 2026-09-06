<script lang="ts">
  import { untrack } from 'svelte';
  import TypeAxis from './TypeAxis.svelte';
  import { inspectorContext } from './context';
  import { inspectFont, isAxisTag, type FontInspection } from 'powermove';
  const { doc } = inspectorContext();
  let { PM, layer, family, onVariableWeight }: { PM: Record<string, any>; layer: any; family: string; onVariableWeight?: (value: boolean) => void } = $props();
  let info = $state<FontInspection | null>(null), attempt = $state(0);
  $effect(() => {
    const selected = family, retry = attempt;
    let active = true;
    info = null; untrack(() => onVariableWeight?.(false));
    void inspectFont(selected, retry > 0).then(result => { if (active) { info = result; onVariableWeight?.(result.axes.some(axis => axis.tag === 'wght')); } });
    return () => { active = false; };
  });
  const saved = $derived((doc.tick.values, doc.tick.history, doc.proj, Object.entries(layer.d ?? {}).flatMap(([key, prop]: [string, any]) => {
    const tag = key.startsWith('fontAxis.') ? key.slice(9) : '';
    if (!isAxisTag(tag) || !prop?.kf || info?.axes.some(axis => axis.tag === tag)) return [];
    return [{ tag, label: tag, default: Number(prop.v) || 0 }];
  })));
</script>
<details class="type-settings" data-font-variations open>
  <summary><span>Type settings</span><span class="font-status">{!info ? 'Loading…' : info.status === 'variable' ? 'Variable' : info.status === 'static' ? 'Static' : 'Unavailable'}</span></summary>
  {#if info?.axes.length}
    {#each info.axes as axis (axis.tag)}<TypeAxis {PM} {layer} {axis} />{/each}
  {:else if info}
    <p class="font-note">{info.status === 'static' ? `${family} is a static font. Choose a variable font to adjust its axes.` : info.status === 'missing' ? `${family} could not be found among installed fonts.` : 'Font details could not be read. Try refreshing the installed fonts.'}</p>
    <button type="button" class="chip refresh-fonts" onclick={() => { attempt++; }}>Refresh fonts</button>
  {/if}
  {#if saved.length}
    <details class="saved-axes"><summary>Saved axis values</summary>
      <p class="font-note">These values remain editable. The selected font has not reported matching axes.</p>
      {#each saved as axis (axis.tag)}<TypeAxis {PM} {layer} {axis} />{/each}
    </details>
  {/if}
</details>
<style>
  .type-settings { margin:6px 0; padding:4px 0; border-top:1px solid var(--line); border-bottom:1px solid var(--line); }
  summary { display:flex; align-items:center; min-height:28px; padding:0 4px; color:var(--tx-2); font-size:var(--fs-xs); cursor:pointer; list-style:none; }
  summary::-webkit-details-marker { display:none; }
  summary::before { content:'›'; width:14px; color:var(--tx-4); }
  details[open] > summary::before { transform:rotate(90deg); }
  .font-status { margin-left:auto; color:var(--tx-4); }
  .font-note { color:var(--tx-4); font-size:var(--fs-xs); line-height:1.5; margin:3px 5px 8px; }
  .refresh-fonts { margin:0 4px 7px; }
  .saved-axes { margin-top:4px; }
</style>
