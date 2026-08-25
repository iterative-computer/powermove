<script lang="ts">
  import type { AgentMessage } from './agent-state.svelte';
  import AttachmentChips from './AttachmentChips.svelte';

  let { PM, message }: { PM: Record<string, any>; message: AgentMessage } = $props();
</script>

<div class:spatial-message={true} class:user={message.role === 'user'} class:assistant={message.role === 'assistant'} class:is-entering={message.entering}>
  {message.text}
  {#if message.attachments?.length}
    <div class="agent-message-files">
      <AttachmentChips {PM} items={message.attachments} />
    </div>
  {/if}
</div>
