<script lang="ts">
  import { onMount } from 'svelte';
  import { kernelSignals } from '../kernel/signals.svelte';
  import type { PanelProps } from './registerSveltePanel';

  interface EffectDefinition {
    label: string;
    group: string;
  }

  let { panelId }: PanelProps = $props();

  const PM = window.PM as Record<string, any>;
  /* Derived over the kernel's effects signal, so an effect registered (or
     replaced, or removed) by an extension shows up without remounting. */
  const groups = $derived.by(() => {
    kernelSignals.effects;
    return Object.entries(PM.FX as Record<string, EffectDefinition>).reduce(
      (result, [id, definition]) => {
        (result[definition.group] ||= []).push([id, definition]);
        return result;
      },
      {} as Record<string, Array<[string, EffectDefinition]>>
    );
  });
  let status = $state('');
  let addedEffect = $state<string | null>(null);
  let feedbackTimer: ReturnType<typeof setTimeout> | undefined;

  function clearFeedback(): void {
    clearTimeout(feedbackTimer);
    feedbackTimer = undefined;
    addedEffect = null;
    status = '';
  }

  onMount(() => {
    const events = ['sel', 'project', 'history'];
    const unsubscribe = events.map(event => PM.bus?.on?.(event, clearFeedback));
    return () => {
      clearTimeout(feedbackTimer);
      events.forEach((event, index) => {
        if (typeof unsubscribe[index] === 'function') unsubscribe[index]();
        else PM.bus?.off?.(event, clearFeedback);
      });
    };
  });

  function addEffect(event: MouseEvent, effect: string, label: string): void {
    /* Native double-click dispatches click, click, dblclick. Preserve the
       legacy guard so one gesture still creates only one effect. */
    if (event.detail > 1) return;
    clearFeedback();
    const layer = PM.firstSel();
    if (!layer) {
      status = 'Select a layer first';
      PM.toast(status);
      return;
    }
    try {
      const result = PM.Edit.apply(
        { type: 'add_effect', target: layer.id, effect },
        { label: `Add ${label}`, origin: 'effects-panel' }
      );
      if (!result?.ok) throw new Error(result?.message || 'The edit was not applied');
    } catch (error) {
      status = `Could not add ${label}: ${error instanceof Error ? error.message : String(error)}`;
      PM.toast(status);
      return;
    }
    PM.Inspector?.refresh?.();
    PM.invalidate();
    status = `Added ${label}`;
    addedEffect = effect;
    PM.toast(`${label} added to ${layer.name || 'selected layer'}`);
    feedbackTimer = setTimeout(clearFeedback, 2200);
  }
</script>

<div class="simple-panel-list" data-svelte-panel={panelId}>
  {#each Object.entries(groups) as [group, effects] (group)}
    <div class="sec simple-panel-section">{group}</div>
    {#each effects as [effect, definition] (effect)}
      <button class="lyr simple-effect-row" class:effect-added={addedEffect === effect} type="button" onclick={(event) => addEffect(event, effect, definition.label)}>
        <span class="nm">{definition.label}</span>
        {#if addedEffect === effect}
          <span class="effect-added-feedback" aria-hidden="true">
            <svg width="12" height="12" viewBox="0 0 16 16" fill="none"><path d="m3 8 3 3 7-7" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" /></svg>
            <span class="effect-added-label">Added</span>
          </span>
        {:else}
          <span class="idx" aria-hidden="true">+</span>
        {/if}
      </button>
    {/each}
  {/each}
  <span class="panel-sr-only" role="status">{status}</span>
</div>

<style>
  .simple-effect-row.effect-added { background: var(--accent-dim); color: var(--tx); }
  .effect-added-feedback { display: flex; align-items: center; gap: 4px; flex: none; color: var(--accent-tx); font-size: var(--fs-xs); }
</style>
