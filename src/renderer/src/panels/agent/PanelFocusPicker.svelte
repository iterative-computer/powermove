<script lang="ts">
  import { onMount } from 'svelte';
  import { agentState, composerMode } from './agent-state.svelte';
  import { panelFocusContext, panelFocusOptions, panelScope, scopePanelIds } from './panel-focus';

  let { PM }: { PM: Record<string, any> } = $props();
  let trigger: HTMLButtonElement;
  let popup: HTMLDivElement;
  let open = $state(false);
  let dismissByPress = false;
  let layoutVersion = $state(0);
  let left = $state(0);
  let top = $state(0);
  const disabled = $derived(composerMode(agentState.legacyPhase).disabled);
  const options = $derived.by(() => {
    void agentState.revision; void layoutVersion;
    return panelFocusOptions(PM.WS?.current, PM.PANELS || {});
  });
  const selected = $derived(scopePanelIds(agentState.scope));
  const focused = $derived(options.filter(option => selected.includes(option.id)));

  onMount(() => {
    const changed = () => { layoutVersion += 1; };
    const off = PM.bus?.on?.('layout:applied', changed);
    const registry = PM.Kernel?.events?.on?.('extensions:changed', changed);
    return () => {
      if (typeof off === 'function') off(); else PM.bus?.off?.('layout:applied', changed);
      registry?.dispose?.();
    };
  });

  $effect(() => {
    void options;
    const current = agentState.scope;
    const normalized = panelFocusContext(current, PM.WS?.current, PM.PANELS || {}).scope;
    if (normalized !== current) queueMicrotask(() => {
      if (agentState.scope === current) PM.AgentUI?.setScope(normalized);
    });
  });

  function close(): void {
    popup.hidePopover();
    trigger.focus();
  }
  function toggle(event: MouseEvent): void {
    const dismiss = open || (event.detail > 0 && dismissByPress);
    dismissByPress = false;
    if (dismiss) { close(); return; }
    const rect = trigger.getBoundingClientRect();
    left = Math.max(8, Math.min(rect.left, window.innerWidth - 256));
    const height = Math.min(300, 88 + options.length * 28);
    top = rect.top >= height + 8 ? rect.top - height - 6 : Math.min(rect.bottom + 6, Math.max(8, window.innerHeight - height - 8));
    popup.showPopover();
    (popup.querySelector<HTMLInputElement>('input:checked') || popup.querySelector<HTMLInputElement>('input'))?.focus();
  }
  function choose(id: string, checked: boolean): void {
    PM.AgentUI?.setScope(panelScope(checked ? [...selected, id] : selected.filter(value => value !== id)));
  }
</script>

<div class="panel-focus-row" aria-label="Focused panels">
  <button class="panel-focus-trigger" type="button" bind:this={trigger} aria-label="Choose focused panels" aria-haspopup="dialog" aria-expanded={open} {disabled} onpointerdown={() => { dismissByPress = open; }}
      onpointercancel={() => { dismissByPress = false; }}
      onclick={toggle}>
    <span>{focused.length === 1 ? focused[0]?.title : focused.length ? `${focused.length} panels` : agentState.scope === 'composition' ? 'Composition content' : 'All panels'}</span>
  </button>
</div>

<div
  class="panel-focus-popup"
  bind:this={popup}
  popover="auto"
  role="dialog"
  tabindex="-1"
  aria-label="Panel focus"
  style:left={`${left}px`}
  style:top={`${top}px`}
  onbeforetoggle={(event) => { open = (event as ToggleEvent).newState === 'open'; }}
  onkeydown={(event) => { event.stopPropagation(); if (event.key === 'Escape') { event.preventDefault(); close(); } }}
>
  <div class="panel-focus-heading"><span>Focus on panels</span></div>
  <div class="panel-focus-list">
    {#each options as panel (panel.id)}
      <label>
        <input type="checkbox" checked={selected.includes(panel.id)} onchange={(event) => choose(panel.id, event.currentTarget.checked)} />
        <span>{panel.title}</span>{#if panel.hidden}<small>Hidden</small>{/if}
      </label>
    {/each}
  </div>
</div>

<style>
  .panel-focus-row { display: flex; align-items: center; min-width: 0; }
  .panel-focus-trigger { display: flex; align-items: center; gap: 4px; min-width: 0; border: 0; border-radius: var(--r-md); padding: 2px 8px; background: transparent; color: var(--tx-3); font: var(--fw-regular) var(--fs-sm)/1.4 var(--f-ui); }
  .panel-focus-trigger span { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .panel-focus-trigger:hover { color: var(--tx); background: var(--ink-1); }
  .panel-focus-trigger:focus-visible { outline: 1px solid var(--accent); outline-offset: 1px; }
  .panel-focus-popup { position: fixed; inset: auto; margin: 0; width: min(248px, calc(100vw - 16px)); max-height: min(300px, calc(100vh - 16px)); box-sizing: border-box; padding: 6px; border: 0; border-radius: var(--r-lg, 12px); background: var(--bg-panel); color: var(--tx); box-shadow: 0 4px 12px -2px rgb(0 0 0 / .16); font: var(--fs-sm)/1.4 var(--f-ui); overflow: auto; }
  .panel-focus-popup::backdrop { background: transparent; }
  .panel-focus-heading { display: flex; align-items: center; justify-content: space-between; padding: 3px 6px 6px; color: var(--tx-3); font-size: var(--fs-xs); }
  .panel-focus-list label { display: flex; align-items: center; gap: 7px; min-height: 28px; padding: 0 6px; border-radius: var(--r-sm); cursor: pointer; }
  .panel-focus-list label:hover { background: var(--ink-1); }
  .panel-focus-list input { margin: 0; width: 13px; height: 13px; accent-color: var(--accent); }
  .panel-focus-list span { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .panel-focus-list small { color: var(--tx-3); font-size: var(--fs-xs); }
</style>
