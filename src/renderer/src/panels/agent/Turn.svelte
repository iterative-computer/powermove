<script lang="ts">
  import type { AgentMessage } from './agent-state.svelte';
  import AttachmentChips from './AttachmentChips.svelte';
  import { activityRows } from './activity-rows';
  import { toRichWords } from './rich-words';

  let { PM, message }: { PM: Record<string, any>; message: AgentMessage } = $props();

  /* Supermove differentiation: the user speaks in a right-aligned bubble; the
     assistant answers as full-width text on the panel itself — no bubble, no
     avatar. New assistant text reveals word by word. */
  const words = $derived(message.role === 'assistant' && message.entering
    ? (message.text ?? '').split(/(\s+)/) : null);

  /* role 'trace': the sealed activity trail of a finished run — same rows as
     the live feed, no transitions, nothing pulses. */
  const traceRows = $derived(message.role === 'trace' ? activityRows(message.steps ?? []) : []);
</script>

{#if message.role === 'trace'}
  <div class="agent-trace is-archived">
    {#each traceRows as row (row.renderKey)}
      {#if row.kind === 'thought'}
        <p class="agent-trace-thought">{#each toRichWords(row.label) as word, wi (wi)}<span class={word.c ? 'agent-trace-code' : ''}>{word.w}</span>{/each}</p>
      {:else if row.kind === 'tools'}
        <div class="agent-trace-tool" class:is-error={row.status === 'error'} title={row.detail?.length ? row.detail.join('\n') : undefined}>
          <svg viewBox="0 0 256 256" aria-hidden="true" focusable="false"><path d="M226.76,69a8,8,0,0,0-12.84-2.88l-40.3,37.19-17.23-3.7-3.7-17.23,37.19-40.3A8,8,0,0,0,187,29.24,72,72,0,0,0,88,96,72.34,72.34,0,0,0,94,124.94L33.79,177.79c-.15.12-.29.26-.43.39a32,32,0,0,0,45.26,45.26c.13-.14.27-.28.39-.43L131.06,162A72,72,0,0,0,232,96,71.56,71.56,0,0,0,226.76,69Zm-71,89a56.14,56.14,0,0,1-21.31-4.18,8,8,0,0,0-9,1.87l-.29.3-57.14,65a16,16,0,0,1-22.62-22.62l65-57.14.3-.29a8,8,0,0,0,1.87-9A56,56,0,0,1,168.72,45.53L138.83,77.9a8,8,0,0,0-1.94,7.1L142.83,113a8,8,0,0,0,6.14,6.14l28,6a8,8,0,0,0,7.1-1.94l32.37-29.89A56.09,56.09,0,0,1,155.76,158Z" /></svg>
          <span>{row.label}</span>
          {#if row.status === 'error'}<em>Failed</em>{/if}
        </div>
      {/if}
    {/each}
  </div>
{:else if message.role === 'user'}
  <div class="agent-msg user" class:is-entering={message.entering}>
    {#if message.focusLabels?.length}<div class="agent-message-focus">Focus · {message.focusLabels.join(', ')}</div>{/if}
    {#if message.attachments?.length}
      <div class="agent-msg-files"><AttachmentChips {PM} items={message.attachments} /></div>
    {/if}
    <div class="agent-bubble">{message.text}</div>
  </div>
{:else}
  <div class="agent-msg assistant" class:is-error={message.error}>
    {#if words}
      <p>{#each words as word, index}<span class="agent-word" style={`--word-index:${Math.min(index, 40)}`}>{word}</span>{/each}</p>
    {:else}
      <p>{message.text}</p>
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
  .agent-message-focus { color: var(--tx-3); font: var(--fs-xs)/1.4 var(--f-ui); text-wrap: pretty; }
</style>
