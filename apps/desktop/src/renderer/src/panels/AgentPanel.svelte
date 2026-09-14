<script lang="ts">
  import { onMount } from 'svelte';
  import { fancySelect } from '../controls/select/enhance';
  import type { ChatGPTAccountStatus } from '../../../shared/ipc';
  import type { PanelProps } from './registerSveltePanel';
  import Composer from './agent/Composer.svelte';
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
  const showConnectionGate = $derived(accountStatus.state !== 'connected' && accountStatus.state !== 'checking');

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

  function changeGateProvider(event: Event): void {
    PM.AgentUI?.setProvider?.((event.currentTarget as HTMLSelectElement).value);
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
    if (scroller) window.requestAnimationFrame(() => scroller?.scrollTo({ top: scroller.scrollHeight }));
  });

  $effect(() => {
    agentState.revision;
    if (!scroller || hasConversationSelection()) return;
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
  {#if showConnectionGate}
    <div class="agent-connect-gate" role="status" aria-live="polite">
      <select class="agent-connect-provider" aria-label="Provider" use:fancySelect={agentState.provider} value={agentState.provider} onchange={changeGateProvider}>
        {#each agentState.providers as provider (provider.id)}
          <option value={provider.id}>{provider.label}</option>
        {/each}
      </select>
      <div class="agent-connect-mark" aria-hidden="true">
        <svg viewBox="0 0 24 24"><path d="M12 3.5c.7 4.6 3.3 7.2 7.9 7.9-4.6.7-7.2 3.3-7.9 7.9-.7-4.6-3.3-7.2-7.9-7.9 4.6-.7 7.2-3.3 7.9-7.9Z" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round" /></svg>
      </div>
      <b>{accountStatus.state === 'connecting' ? `Finish connecting ${providerName}` : `Connect ${providerName}`}</b>
      <span>{accountStatus.detail || (accountStatus.state === 'unavailable'
        ? `Powermove could not start its ${providerName} service.`
        : `Use your ${providerName} subscription to power the Powermove agent.`)}</span>
      <button
        class="btn pri agent-connect-button"
        type="button"
        disabled={accountBusy || accountStatus.state === 'connecting'}
        onclick={accountStatus.state === 'unavailable' ? retryProvider : connectProvider}
      >{accountStatus.state === 'connecting' ? 'Waiting…' : accountStatus.state === 'unavailable' ? 'Try again' : `Connect ${providerName}`}</button>
      <button type="button" class="btn" onclick={() => PM.SettingsUI?.open('accounts')}>All connection options</button>
    </div>
  {/if}
    <!-- svelte-ignore a11y_no_noninteractive_tabindex (The scrollable transcript needs keyboard focus for text selection and scrolling.) -->
    <div
      class="agent-scroll"
      data-native-text
      tabindex="0"
      onpointerdown={() => { selectingText = true; }}
      role="log"
      aria-label="Agent conversation"
      aria-live="polite"
      aria-atomic="false"
      bind:this={scroller}
      onscroll={onScroll}
      data-overflow-bottom={showJump ? '1' : '0'}
    >
      {#key agentState.threadId}<Conversation {PM} />{/key}
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

<style>
  .agent-connect-gate { flex: 0 1 auto; padding: 8px 12px; }
  .agent-connect-mark { display: none; }
  .agent-connect-button { margin-top: 8px; }
  /* The set-height variable is updated by explicit splitter resizing. The
     important basis also pins older live sessions whose inline style is fluid. */
  :global(#panel-agent:not([data-collapsed="1"])) {
    flex: 0 0 var(--set-panel-height, 350px) !important;
  }
</style>
