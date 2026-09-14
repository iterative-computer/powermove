<script lang="ts">
  import ErrorNotice from '../../errors/ErrorNotice.svelte';

  import { onMount } from 'svelte';
  import { agentState } from './agent-state.svelte';
  import type { AgentMessage } from './agent-state.svelte';
  import AttachmentChips from './AttachmentChips.svelte';
  import { activityRows } from './activity-rows';
  import { sendMessage } from './text-reveal';
  import Markdown from './Markdown.svelte';
  import { mountPromptGlow } from './prompt-glow';
  import { glowFade } from './motion';
  import ModResult from './ModResult.svelte';
  import { modResultForMessage } from './mod-result';
  import TextRow from './TextRow.svelte';
  import ToolActivity from './ToolActivity.svelte';

  let {
    PM,
    message,
    answering = false,
    messageIndex,
  }: { PM: Record<string, any>; message: AgentMessage; answering?: boolean; messageIndex?: number } = $props();

  let modRevision = $state(0);
  onMount(() => {
    const subscription = PM.Kernel?.events?.on?.('extensions:changed', () => { modRevision += 1; });
    return () => subscription?.dispose?.();
  });
  const modResult = $derived.by(() => {
    void modRevision;
    return modResultForMessage(message, PM.Kernel?.panels?.entries?.() || []);
  });
  function promptSignal(node: HTMLElement) { return { destroy: mountPromptGlow(node) }; }

  /* role 'trace': the sealed activity trail of a finished run — same rows as
     the live feed, no transitions, nothing pulses. */
  const traceRows = $derived(message.role === 'trace' ? activityRows(message.steps ?? []) : []);
</script>

{#if message.role === 'trace'}
  <div class="agent-trace is-archived">
    {#each traceRows as row (row.renderKey)}
      {#if row.kind === 'text'}
        <TextRow text={row.text} />
      {:else if row.kind === 'tools'}
        <ToolActivity {row} />
      {/if}
    {/each}
  </div>
{:else if message.role === 'user'}
  <div class="agent-msg user" class:is-entering={message.entering} class:is-steering={message.steering}>
    {#if message.focusLabels?.length}<div class="agent-message-focus">Focus · {message.focusLabels.join(', ')}</div>{/if}
    {#if message.attachments?.length}
      <div class="agent-msg-files"><AttachmentChips {PM} items={message.attachments} /></div>
    {/if}
    <div class="agent-prompt" class:is-answering={answering} use:sendMessage={Boolean(message.entering)}>
      {#if answering}<div class="agent-prompt-signal" data-prompt-halo aria-hidden="true" use:promptSignal out:glowFade={{duration: 220}}></div>{/if}
      <div class="agent-bubble">{message.text}</div>
    </div>
  </div>
{:else}
  <div class="agent-msg assistant" class:is-error={message.error}>
    {#if message.error}
      <ErrorNotice error={message.text} live={Boolean(message.entering)}>
        {#snippet actions()}
          <button type="button" class="btn" disabled={agentState.phase === 'running'} onclick={() => PM.AgentUI?.retry?.(messageIndex)}>Try again</button>
        {/snippet}
      </ErrorNotice>
    {:else if modResult}
      <ModResult {PM} result={modResult} />
    {:else}
      <div class="agent-reply"><Markdown text={message.text || ''} streaming={Boolean(message.entering)} animated={Boolean(message.entering)} /></div>
      {#if message.requiresProject && messageIndex !== undefined && agentState.provider !== 'compatible'}
        <button type="button" class="btn agent-error-retry"
          disabled={agentState.phase === 'running'}
          onclick={() => PM.AgentUI?.continueWithProject?.(messageIndex)}>Continue with Project access</button>
      {/if}
    {/if}
    {#if message.attachments?.length}
      <div class="agent-msg-files"><AttachmentChips {PM} items={message.attachments} /></div>
    {/if}
  </div>
{/if}

<style>
  .agent-prompt-signal{position:absolute;inset:-8px;overflow:hidden;border-radius:22px 22px 12px 22px;pointer-events:none;z-index:0}
  .agent-prompt-signal :global(canvas){opacity:.8}
  .agent-prompt-signal::before{content:"";position:absolute;inset:8px;border-radius:14px 14px 4px 14px;box-shadow:0 0 8px color-mix(in srgb,var(--accent) 20%,transparent)}
  .agent-prompt-signal:global([data-glow-renderer="motion-gpu"])::before{display:none}
  .agent-message-focus { color: var(--tx-3); font: var(--fs-xs)/1.4 var(--f-ui); text-wrap: pretty; }
  /* Steering reads as a continuation of the request above it, not a new turn. */
  .agent-msg.user.is-steering { margin-top: -12px; }
</style>
