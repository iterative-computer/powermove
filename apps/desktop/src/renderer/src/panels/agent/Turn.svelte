<script lang="ts">
  import ErrorNotice from '../../errors/ErrorNotice.svelte';

  import { onMount } from 'svelte';
  import type { AgentMessage } from './agent-state.svelte';
  import AttachmentChips from './AttachmentChips.svelte';
  import { activityRows } from './activity-rows';
  import { toRichWords } from './rich-words';
  import { revealText, sendMessage } from './text-reveal';
  import { mountPromptGlow } from './prompt-glow';
  import { glowFade } from './motion';
  import ModResult from './ModResult.svelte';
  import { modResultForMessage } from './mod-result';
  import ToolActivity from './ToolActivity.svelte';

  let {
    PM,
    message,
    answering = false,
  }: { PM: Record<string, any>; message: AgentMessage; answering?: boolean } = $props();

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
      {#if row.kind === 'thought'}
        <p class="agent-trace-thought">{#each toRichWords(row.label) as word, wi (wi)}<span class={word.c ? 'agent-trace-code' : ''}>{word.w}</span>{/each}</p>
      {:else if row.kind === 'text'}
        <p class="agent-trace-text">{#each toRichWords(row.text) as word, wi (wi)}<span class={word.c ? 'agent-trace-code' : ''}>{word.w}</span>{/each}</p>
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
      <ErrorNotice error={message.text} live={Boolean(message.entering)} />
    {:else if modResult}
      <ModResult {PM} result={modResult} />
    {:else}
      <p>{#each (message.text || '').split(/(\s+)/) as word, index (index)}<span use:revealText={Boolean(message.entering) && Boolean(word.trim())}>{word}</span>{/each}</p>
    {/if}
    {#if message.fixExtensionId}
      <div class="agent-card-actions">
        <button class="agent-btn" type="button" onclick={() => PM.SpatialAssistant?.requestFix?.(message.fixExtensionId)}>Fix it</button>
      </div>
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
