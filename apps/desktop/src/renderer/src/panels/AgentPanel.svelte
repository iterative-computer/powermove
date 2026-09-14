<script lang="ts">
  import { onMount } from 'svelte';
  import type { ChatGPTAccountStatus } from '../../../shared/ipc';
  import type { PanelProps } from './registerSveltePanel';
  import Composer from './agent/Composer.svelte';
  import ConnectionGate from './agent/ConnectionGate.svelte';
  import Conversation from './agent/Conversation.svelte';
  import PlanPreview from './agent/PlanPreview.svelte';
  import ResultActions from './agent/ResultActions.svelte';
  import ThreadPicker from './agent/ThreadPicker.svelte';
  import { agentState } from './agent/agent-state.svelte';

  let { panelId }: PanelProps = $props();
  const PM = window.PM as Record<string, any>;

  /* The transcript owns its vertical scroll. Only edits awaiting review belong
     in the footer; autonomous replies and their files stay in the thread. */
  let scroller = $state<HTMLDivElement>();
  let showJump = $state(false);
  let userScrolled = false;
  let selectingText = false;
  function hasConversationSelection(): boolean {
    const selection = window.getSelection();
    return selectingText || !!(selection && !selection.isCollapsed && scroller?.contains(selection.anchorNode));
  }
  let accountStatus = $state<ChatGPTAccountStatus>({
    state: 'checking', email: null, planType: null, detail: null
  });
  let accountBusy = $state(false);
  const providerName = $derived(agentState.provider === 'compatible' ? 'API or local model' : agentState.provider === 'claude' ? 'Claude' : 'ChatGPT');

  const showPreview = $derived(agentState.phase === 'preview');
  const showResult = $derived(agentState.phase === 'result' && (agentState.panelRun || !agentState.run?.autonomous));
  // Keep setup mounted when switching providers or retrying a connection so
  // the composer does not flash between asynchronous status responses.
  let connectionRequired = $state(false);
  $effect(() => {
    if (accountStatus.state !== 'checking') connectionRequired = accountStatus.state !== 'connected';
  });
  const showConnectionGate = $derived(accountStatus.state === 'checking' ? connectionRequired : accountStatus.state !== 'connected');
  const showSetup = $derived(showConnectionGate && agentState.phase === 'idle' && !agentState.conversation.length && !agentState.activity);

  onMount(() => {
    // Panels mount before app.ts chooses the boot project. Synchronize on the
    // first frame so saved threads are present before the composer is usable.
    const frame = window.requestAnimationFrame(() => PM.AgentUI?.update?.({ flush: true }));
    const endSelection = () => { selectingText = false; };
    window.addEventListener('pointerup', endSelection);
    window.addEventListener('pointercancel', endSelection);
    return () => { window.cancelAnimationFrame(frame); window.removeEventListener('pointerup', endSelection); window.removeEventListener('pointercancel', endSelection); };
  });

  function unavailable(error: unknown): ChatGPTAccountStatus {
    return {
      state: 'unavailable', email: null, planType: null,
      detail: error instanceof Error ? error.message : 'ChatGPT could not be reached.'
    };
  }

  function providerApi() {
    return agentState.provider === 'claude' ? window.powermove?.claude : window.powermove?.chatgpt;
  }

  async function connectProvider(): Promise<void> {
    if (agentState.provider === 'compatible') { PM.SettingsUI?.open('accounts'); return; }
    const api = providerApi();
    if (!api || accountBusy) return;
    const provider = agentState.provider;
    accountBusy = true;
    accountStatus = {
      state: 'connecting', email: null, planType: null, detail: `Opening ${providerName} sign-in…`
    };
    try {
      const status = await api.connect();
      if (provider === agentState.provider) accountStatus = status;
    } catch (error) {
      if (provider === agentState.provider) accountStatus = unavailable(error);
    } finally {
      if (provider === agentState.provider) accountBusy = false;
    }
  }

  async function retryProvider(): Promise<void> {
    if (agentState.provider === 'compatible') { PM.SettingsUI?.open('accounts'); return; }
    const api = providerApi();
    if (!api || accountBusy) return;
    const provider = agentState.provider;
    accountBusy = true;
    accountStatus = { state: 'checking', email: null, planType: null, detail: null };
    try {
      const status = await api.status();
      if (provider === agentState.provider) accountStatus = status;
    } catch (error) {
      if (provider === agentState.provider) accountStatus = unavailable(error);
    } finally {
      if (provider === agentState.provider) accountBusy = false;
    }
  }

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
    scroller?.scrollTo({ top: scroller.scrollHeight, behavior: window.matchMedia?.('(prefers-reduced-motion: reduce)').matches ? 'instant' : 'smooth' });
  }

  /* Pin-to-bottom is a threshold, not a boolean stick: while the reader is
     within 160px of the end, new content keeps them at the end. */
  $effect(() => {
    agentState.threadId;
    userScrolled = false;
    showJump = false;
    if (scroller && !showSetup) window.requestAnimationFrame(() => scroller?.scrollTo({ top: scroller.scrollHeight }));
  });

  $effect(() => {
    agentState.revision;
    if (!scroller || showSetup || hasConversationSelection()) return;
    if (!userScrolled || distanceFromBottom() < 160) {
      const el = scroller;
      window.requestAnimationFrame(() => { if (!hasConversationSelection()) el.scrollTo({ top: el.scrollHeight }); });
    }
  });

  $effect(() => {
    const provider = agentState.provider;
    let alive = true;
    accountBusy = false;
    accountStatus = { state: 'checking', email: null, planType: null, detail: null };
    if (provider === 'compatible') {
      const update = () => window.powermove?.compatible?.status().then(config => {
        if (alive) accountStatus = { state: config.model ? 'connected' : 'disconnected', email: null, planType: null,
          detail: config.model ? null : 'Connect an API, Ollama, or LM Studio in settings.' };
      }, error => { if (alive) accountStatus = unavailable(error); });
      void update();
      window.addEventListener('pm-provider-connected', update);
      return () => { alive = false; window.removeEventListener('pm-provider-connected', update); };
    }
    const api = provider === 'claude' ? window.powermove?.claude : window.powermove?.chatgpt;
    if (!api) {
      accountStatus = unavailable(new Error(`Restart Powermove to finish installing ${provider === 'claude' ? 'Claude' : 'ChatGPT'} connection.`));
      return;
    }
    const stop = api.onChanged((status) => { if (alive) accountStatus = status; });
    void api.status().then(
      (status) => { if (alive) accountStatus = status; },
      (error) => { if (alive) accountStatus = unavailable(error); }
    );
    return () => { alive = false; stop(); };
  });
</script>

<div class="agent-panel-body agent-shell" data-svelte-panel={panelId} data-agent-panel data-agent-phase={agentState.phase}>
  <ThreadPicker {PM} />
  <!-- svelte-ignore a11y_no_noninteractive_tabindex (The scrollable transcript needs keyboard focus for text selection and scrolling.) -->
  <div
    class="agent-scroll"
    class:agent-setup-scroll={showSetup}
    data-native-text
    tabindex="0"
    onpointerdown={() => { selectingText = true; }}
    role={showSetup ? 'region' : 'log'}
    aria-label={showSetup ? 'Agent setup' : 'Agent conversation'}
    aria-live="polite"
    aria-atomic="false"
    bind:this={scroller}
    onscroll={onScroll}
    data-overflow-bottom={showJump ? '1' : '0'}
  >
    {#if showConnectionGate}
      <ConnectionGate {PM} status={accountStatus} busy={accountBusy} {providerName}
        compact={!showSetup} connect={connectProvider} retry={retryProvider} />
    {/if}
    {#if !showSetup}{#key agentState.threadId}<Conversation {PM} />{/key}{/if}
  </div>
  {#if !showSetup}
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
  {/if}
</div>

<style>
  .agent-setup-scroll { padding: 10px 16px 16px; mask-image: none; }
  /* The set-height variable is updated by explicit splitter resizing. The
     important basis also pins older live sessions whose inline style is fluid. */
  :global(#panel-agent:not([data-collapsed="1"])) {
    flex: 0 0 var(--set-panel-height, 350px) !important;
  }
</style>
