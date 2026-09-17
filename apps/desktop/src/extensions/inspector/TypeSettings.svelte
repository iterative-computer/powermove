<script lang="ts">
  import { untrack } from 'svelte';
  import TypeAxis from './TypeAxis.svelte';
  import { inspectorContext } from './context';
  import { inspectFont, isAxisTag, type FontInspection } from 'powermove';

  /* Variable font axes, in the inspector's own row grammar. A variable font
     shows one row per axis under a quiet "Type settings" heading. A static
     font has nothing to set, so the section stays out of the way: only saved
     axis values (from a font that used to be variable) keep it visible. A
     font that cannot be read gets a single status row with a refresh action. */

  const { doc } = inspectorContext();
  let { layer, family, onVariableWeight }: { layer: any; family: string; onVariableWeight?: (value: boolean) => void } = $props();
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
  const unreadable = $derived(!!info && info.status !== 'variable' && info.status !== 'static');
  const shown = $derived(!!info && (info.axes.length > 0 || saved.length > 0 || unreadable));
</script>

{#if shown}
  <div class="type-settings" data-font-variations data-font-status={info?.status}>
    <div class="sec type-settings-head">
      <span>Type settings</span>
      {#if unreadable}
        <button type="button" class="type-settings-action" onclick={() => { attempt++; }}>Refresh fonts</button>
      {/if}
    </div>
    {#each info?.axes ?? [] as axis (axis.tag)}<TypeAxis {layer} {axis} />{/each}
    {#each saved as axis (axis.tag)}<TypeAxis {layer} {axis} />{/each}
    {#if unreadable}
      <div class="row split">
        <div class="k">Font</div>
        <div class="vwrap"><span class="type-settings-status">{info?.status === 'missing' ? 'Not installed' : 'Could not be read'}</span></div>
      </div>
    {/if}
  </div>
{/if}

<style>
  .type-settings-head { display: flex; align-items: center; justify-content: space-between; }
  .type-settings-action { padding: 0; border: 0; background: transparent; color: var(--tx-3); font: inherit; font-size: var(--fs-xs); font-weight: var(--fw-regular); cursor: default; }
  .type-settings-action:hover { color: var(--tx); }
  .type-settings-status { color: var(--tx-3); font-size: var(--fs-xs); }
</style>
