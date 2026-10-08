<script lang="ts">
  import ErrorNotice from '../../errors/ErrorNotice.svelte';

  import { onMount } from 'svelte';
  import { agentState } from './agent-state.svelte';
  import type { AgentMessage } from './agent-state.svelte';
  import AttachmentChips from './AttachmentChips.svelte';
  import { activityRows } from './activity-rows';
  import Markdown from './Markdown.svelte';
  import { mountPromptGlow } from './prompt-glow';
  import { glowFade } from './motion';
  import ModResult from './ModResult.svelte';
  import { modResultForMessage } from './mod-result';
  import QuestionCard from './QuestionCard.svelte';
  import TextRow from './TextRow.svelte';
  import ThoughtRow from './ThoughtRow.svelte';
  import ToolActivity from './ToolActivity.svelte';
  import { splitUIPlacementText } from './ui-placement';
  import { displayPromptText, promptSegments } from './inline-prompt';
  import { activatePromptAttachment } from './attachments';

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
  // Sent files sit where they were placed in the composer, not in a rail above.
  const prompt = $derived(message.role === 'user'
    ? promptSegments(message.text || '', (message.attachments ?? []) as Array<Record<string, any> & { name: string }>)
    : { segments: [], loose: [] });
  function promptSignal(node: HTMLElement) { return { destroy: mountPromptGlow(node) }; }

  /* A turn that failed is not automatically the editor's error. Messages
     written before the distinction existed carry no kind and stay errors. */
  const notice = $derived(message.notice ?? 'error');

  // Archiving must preserve the same prose and ordering as the live timeline.
  // Only individual tool groups disclose their details; a follow-up must not
  // turn the preceding response into a last-paragraph summary.
  const traceRows = $derived(activityRows(message.role === 'trace' ? message.steps ?? [] : []));
</script>

{#snippet workRows()}
  {#each traceRows as row (row.renderKey)}
    {#if row.kind === 'text'}
      <TextRow text={row.text} />
    {:else if row.kind === 'thought'}
      <ThoughtRow text={row.text} />
    {:else if row.kind === 'question'}
      <QuestionCard {PM} step={row} />
    {:else if row.kind === 'tools'}
      <ToolActivity {row} />
    {/if}
  {/each}
{/snippet}

{#if message.role === 'trace'}
  <div class="agent-trace is-archived">
    {@render workRows()}
  </div>
{:else if message.role === 'user'}
  <div class="agent-msg user" class:is-entering={message.entering} class:is-steering={message.steering}>
    {#if message.focusLabels?.length}<div class="agent-message-focus">Focus · {message.focusLabels.join(', ')}</div>{/if}
    {#if prompt.loose.length}
      <div class="agent-msg-files"><AttachmentChips {PM} items={prompt.loose} /></div>
    {/if}
    <div class="agent-prompt" class:is-answering={answering}>
      {#if answering}<div class="agent-prompt-signal" data-prompt-halo aria-hidden="true" use:promptSignal out:glowFade={{duration: 220}}></div>{/if}
      <div class="agent-bubble">{#if prompt.segments.some(segment => 'attachment' in segment)}{#each prompt.segments as segment, index (index)}{#if 'text' in segment}<div class="agent-prompt-segment"><Markdown text={segment.text} /></div>{:else}<button
        type="button" class="agent-inline-attachment is-sent"
        aria-label={segment.attachment.dataUrl ? `View ${segment.attachment.name}` : `Reveal ${segment.attachment.name} in its folder`}
        title={segment.attachment.name}
        onclick={() => void activatePromptAttachment(PM, segment.attachment)}
      >{#if segment.attachment.dataUrl}<img src={segment.attachment.dataUrl} alt="" />{:else}<span class="agent-inline-attachment-type" aria-hidden="true">{(segment.attachment.name.split('.').pop() || 'file').slice(0, 5).toUpperCase()}</span>{/if}<span>{segment.attachment.name.replace(/\.[^.]+$/, '') || segment.attachment.name}</span></button>{/if}{/each}{:else}<Markdown text={displayPromptText(message.text || '') || (message.attachments?.length ? `Attached ${message.attachments.length} file${message.attachments.length === 1 ? '' : 's'}` : '')} />{/if}</div>
    </div>
  </div>
{:else}
  <div class="agent-msg assistant" class:is-error={message.error && notice === 'error'}>
    {#if message.error && notice !== 'plain'}
      <ErrorNotice markdown error={message.text} live={Boolean(message.entering)} kind={notice}>
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
  .agent-prompt-segment { display: contents; }
  .agent-prompt-segment :global(.agent-md-p:only-child) { display: inline; }
  .agent-prompt-signal{position:absolute;inset:-8px;overflow:hidden;border-radius:22px 22px 12px 22px;pointer-events:none;z-index:0}
  .agent-prompt-signal :global(canvas){opacity:.8}
  .agent-prompt-signal::before{content:"";position:absolute;inset:8px;border-radius:14px 14px 4px 14px;box-shadow:0 0 8px color-mix(in srgb,var(--accent) 20%,transparent)}
  .agent-prompt-signal:global([data-glow-renderer="motion-gpu"])::before{display:none}
  .agent-message-focus { color: var(--tx-3); font: var(--fs-xs)/1.4 var(--f-ui); text-wrap: pretty; }
  /* Steering reads as a continuation of the request above it, not a new turn. */
  .agent-msg.user.is-steering { margin-top: -12px; }
</style>
