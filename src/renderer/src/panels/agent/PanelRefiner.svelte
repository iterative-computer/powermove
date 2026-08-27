<script lang="ts">
  import Composer from './Composer.svelte';
  import Timeline from './Timeline.svelte';
  import PlanPreview from './PlanPreview.svelte';
  import ResultActions from './ResultActions.svelte';
  import Turn from './Turn.svelte';
  import { agentState } from './agent-state.svelte';
  let { PM, panelId, close }: { PM: Record<string, any>; panelId: string; close: () => void } = $props();
  const lastReply = $derived(agentState.conversation.at(-1));
</script>

<section class="panel-refiner agent-shell" aria-label={`Refine ${PM.PANELS[panelId]?.title || panelId}`}>
  <div class="panel-refiner-heading"><span>Refine this panel</span><button type="button" onclick={close} aria-label="Close panel composer">×</button></div>
  <div class="panel-refiner-response">
    {#if lastReply?.role === 'assistant'}<Turn {PM} message={lastReply} />{/if}
    {#if agentState.phase === 'running'}<Timeline {PM} />{/if}
    {#if agentState.phase === 'preview'}<PlanPreview {PM} />{/if}
    {#if agentState.phase === 'result'}<ResultActions {PM} />{/if}
  </div>
  <Composer {PM} panelId={`refine-${panelId}`} />
</section>

<style>
  .panel-refiner { flex: none; min-width: 0; padding: 8px; gap: 8px; border-top: 1px solid var(--line); background: var(--bg-panel); }
  .panel-refiner-heading { display: flex; align-items: center; justify-content: space-between; color: var(--tx-3); font: var(--fs-sm)/1.4 var(--f-ui); }
  .panel-refiner-heading button { background: transparent; color: inherit; border: 0; font-size: 17px; line-height: 1; }
  .panel-refiner-response { max-height: 140px; overflow: auto; }
  .panel-refiner-response:empty { display: none; }
</style>
