<script lang="ts">
  import type { AgentMessage } from './agent-state.svelte';
  import AttachmentChips from './AttachmentChips.svelte';

  let { PM, message }: { PM: Record<string, any>; message: AgentMessage } = $props();

  /* Supermove differentiation: the user speaks in a right-aligned bubble; the
     assistant answers as full-width text on the panel itself — no bubble, no
     avatar. New assistant text reveals word by word. */
  const words = $derived(message.role === 'assistant' && message.entering
    ? message.text.split(/(\s+)/) : null);
</script>

{#if message.role === 'user'}
  <div class="agent-msg user" class:is-entering={message.entering}>
    {#if message.attachments?.length}
      <div class="agent-msg-files"><AttachmentChips {PM} items={message.attachments} /></div>
    {/if}
    <div class="agent-bubble">{message.text}</div>
  </div>
{:else}
  <div class="agent-msg assistant">
    {#if words}
      <p>{#each words as word, index}<span class="agent-word" style={`--word-index:${Math.min(index, 40)}`}>{word}</span>{/each}</p>
    {:else}
      <p>{message.text}</p>
    {/if}
    {#if message.attachments?.length}
      <div class="agent-msg-files"><AttachmentChips {PM} items={message.attachments} /></div>
    {/if}
  </div>
{/if}
