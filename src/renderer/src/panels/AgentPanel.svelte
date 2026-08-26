<script lang="ts">
  import type { PanelProps } from './registerSveltePanel';
  import Composer from './agent/Composer.svelte';
  import Conversation from './agent/Conversation.svelte';
  import PlanPreview from './agent/PlanPreview.svelte';
  import ResultActions from './agent/ResultActions.svelte';
  import { agentState } from './agent/agent-state.svelte';

  let { panelId }: PanelProps = $props();
  const PM = window.PM as Record<string, any>;

  /* Supermove layout: the thread scrolls under a mask fade; proposals, results
     and the composer live in a floating footer the scroller never runs under. */
  let scroller = $state<HTMLDivElement>();
  let showJump = $state(false);
  let userScrolled = false;

  const showPreview = $derived(agentState.phase === 'preview');
  const showResult = $derived(agentState.phase === 'result');

  function distanceFromBottom(): number {
    if (!scroller) return 0;
    return scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight;
  }

  function onScroll(): void {
    const away = distanceFromBottom() > 48;
    showJump = away;
    if (away) userScrolled = true;
    else userScrolled = false;
  }

  function jumpToLatest(): void {
    userScrolled = false;
    scroller?.scrollTo({ top: scroller.scrollHeight, behavior: 'smooth' });
  }

  /* Pin-to-bottom is a threshold, not a boolean stick: while the reader is
     within 160px of the end, new content keeps them at the end. */
  $effect(() => {
    agentState.revision;
    if (!scroller) return;
    if (!userScrolled || distanceFromBottom() < 160) {
      const el = scroller;
      window.requestAnimationFrame(() => el.scrollTo({ top: el.scrollHeight }));
    }
  });
</script>

<div class="agent-panel-body agent-shell" data-svelte-panel={panelId} data-agent-panel data-agent-phase={agentState.phase}>
  <div
    class="agent-scroll"
    role="log"
    aria-label="Agent conversation"
    aria-live="polite"
    aria-atomic="false"
    bind:this={scroller}
    onscroll={onScroll}
  >
    <Conversation {PM} />
  </div>
  <div class="agent-footer">
    {#if showJump}
      <button class="agent-jump" type="button" aria-label="Scroll to latest" onclick={jumpToLatest}>
        <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M4 6.5 8 10.5 12 6.5" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" /></svg>
      </button>
    {/if}
    {#if showPreview}<PlanPreview {PM} />{/if}
    {#if showResult}<ResultActions {PM} />{/if}
    <Composer {PM} {panelId} />
  </div>
</div>
