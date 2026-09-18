<script lang="ts">
  import ErrorNotice from '../../errors/ErrorNotice.svelte';

  import { onMount } from 'svelte';
  import { agentState } from './agent-state.svelte';
  import type { AgentMessage } from './agent-state.svelte';
  import AttachmentChips from './AttachmentChips.svelte';
  import { activityRows, durationLabel } from './activity-rows';
  import Markdown from './Markdown.svelte';
  import { mountPromptGlow } from './prompt-glow';
  import { glowFade } from './motion';
  import ModResult from './ModResult.svelte';
  import { modResultForMessage } from './mod-result';
  import TextRow from './TextRow.svelte';
  import ToolActivity from './ToolActivity.svelte';
  import { splitUIPlacementText } from './ui-placement';

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

  /* A turn that failed is not automatically the editor's error. Messages
     written before the distinction existed carry no kind and stay errors. */
  const notice = $derived(message.notice ?? 'error');

  // Autonomous replies are stored as trace text. Keep the last reply outside
  // the disclosure so collapsing the work never hides the agent's answer.
  const steps = $derived(message.role === 'trace' ? message.steps ?? [] : []);
  const replyIndex = $derived(steps.reduce((last, step, index) => step.kind === 'text' && step.text.trim() ? index : last, -1));
  const reply = $derived(replyIndex >= 0 ? steps[replyIndex] : undefined);
  const traceRows = $derived(activityRows(steps.filter((_, index) => index !== replyIndex).map(step =>
    step.kind === 'thought' ? { kind: 'text' as const, id: step.id, text: step.label } : step)));
  const workedFor = $derived.by(() => {
    if (Number.isFinite(message.durationMs) && message.durationMs! >= 0) return durationLabel(message.durationMs!);
    // Older saved conversations only have timestamps on their tool/thought steps.
    const starts = steps.flatMap(step => step.kind !== 'text' && Number.isFinite(step.startedAt) ? [step.startedAt!] : []);
    const ends = steps.flatMap(step => step.kind !== 'text' && Number.isFinite(step.endedAt) ? [step.endedAt!] : []);
    return starts.length && ends.length ? durationLabel(Math.max(...ends) - Math.min(...starts)) : '';
  });
</script>

{#if message.role === 'trace'}
  <div class="agent-trace is-archived">
    {#if traceRows.length}
      <details class="agent-work-log">
        <summary><span>{workedFor ? `Worked for ${workedFor}` : 'Worked'}</span><svg viewBox="0 0 16 16" aria-hidden="true"><path d="m6 4 4 4-4 4" /></svg></summary>
        <div class="agent-work-details" aria-label="Thinking and activity">
          {#each traceRows as row (row.renderKey)}
            {#if row.kind === 'text'}
              <TextRow text={row.text} />
            {:else if row.kind === 'tools'}
              <ToolActivity {row} />
            {/if}
          {/each}
        </div>
      </details>
    {/if}
    {#if reply?.kind === 'text'}<TextRow text={reply.text} />{/if}
  </div>
{:else if message.role === 'user'}
  <div class="agent-msg user" class:is-entering={message.entering} class:is-steering={message.steering}>
    {#if message.focusLabels?.length}<div class="agent-message-focus">Focus · {message.focusLabels.join(', ')}</div>{/if}
    {#if message.attachments?.length}
      <div class="agent-msg-files"><AttachmentChips {PM} items={message.attachments} /></div>
    {/if}
    <div class="agent-prompt" class:is-answering={answering}>
      {#if answering}<div class="agent-prompt-signal" data-prompt-halo aria-hidden="true" use:promptSignal out:glowFade={{duration: 220}}></div>{/if}
      <div class="agent-bubble">{message.text}</div>
    </div>
  </div>
{:else}
  <div class="agent-msg assistant" class:is-error={message.error && notice === 'error'}>
    {#if message.error && notice !== 'plain'}
      <ErrorNotice error={message.text} live={Boolean(message.entering)} kind={notice}>
        {#snippet actions()}
          <button type="button" class="btn" disabled={agentState.phase === 'running'} onclick={() => PM.AgentUI?.retry?.(messageIndex)}>Try again</button>
        {/snippet}
      </ErrorNotice>
    {:else if message.error}
      <!-- Nothing broke and nothing was lost: the agent is simply answering,
           so the turn reads like any other reply, with the retry it still
           deserves. -->
      <div class="agent-reply"><Markdown text={message.text || ''} streaming={Boolean(message.entering)} animated={Boolean(message.entering)} /></div>
      <button type="button" class="btn agent-error-retry"
        disabled={agentState.phase === 'running'}
        onclick={() => PM.AgentUI?.retry?.(messageIndex)}>Try again</button>
    {:else if modResult}
      <ModResult {PM} result={modResult} />
    {:else}
      <div class="agent-reply"><Markdown text={splitUIPlacementText(message.text || '').text} streaming={Boolean(message.entering)} animated={Boolean(message.entering)} /></div>
      {#if message.requiresProject && messageIndex !== undefined}
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
  .agent-work-log { min-width: 0; color: var(--tx-3); font-size: var(--fs-sm); border-bottom: 1px solid var(--line); padding-bottom: 6px; }
  .agent-work-log > summary { display: flex; align-items: center; gap: 6px; width: fit-content; max-width: 100%; padding: 3px 4px 3px 0; list-style: none; line-height: 1.5; border-radius: var(--r-sm); }
  .agent-work-log > summary::-webkit-details-marker { display: none; }
  .agent-work-log > summary:hover { color: var(--tx); }
  .agent-work-log > summary:focus-visible { outline: 2px solid var(--accent); outline-offset: 3px; }
  .agent-work-log svg { width: 12px; height: 12px; flex: none; fill: none; stroke: currentColor; stroke-width: 1.3; stroke-linecap: round; stroke-linejoin: round; transition: transform var(--dur-2); }
  .agent-work-log[open] > summary svg { transform: rotate(90deg); }
  .agent-work-details { display: flex; flex-direction: column; gap: 10px; padding: 10px 0 4px; min-width: 0; }
  @media (prefers-reduced-motion: reduce) { .agent-work-log svg { transition: none; } }
  .agent-prompt-signal{position:absolute;inset:-8px;overflow:hidden;border-radius:22px 22px 12px 22px;pointer-events:none;z-index:0}
  .agent-prompt-signal :global(canvas){opacity:.8}
  .agent-prompt-signal::before{content:"";position:absolute;inset:8px;border-radius:14px 14px 4px 14px;box-shadow:0 0 8px color-mix(in srgb,var(--accent) 20%,transparent)}
  .agent-prompt-signal:global([data-glow-renderer="motion-gpu"])::before{display:none}
  .agent-message-focus { color: var(--tx-3); font: var(--fs-xs)/1.4 var(--f-ui); text-wrap: pretty; }
  /* Steering reads as a continuation of the request above it, not a new turn. */
  .agent-msg.user.is-steering { margin-top: -12px; }
</style>
