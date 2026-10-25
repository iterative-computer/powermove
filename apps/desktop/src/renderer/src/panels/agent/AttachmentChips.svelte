<script lang="ts">
  import Icon from '../Icon.svelte';
  import { activatePromptAttachment } from './attachments';

  let {
    PM,
    items,
    removable = false,
    onRemove
  }: {
    PM: Record<string, any>;
    items: Array<Record<string, any> | string>;
    removable?: boolean;
    onRemove?: (id: string) => void;
  } = $props();

  const normalized = $derived(items.map((item) => typeof item === 'string' ? { name: item } : item));
</script>

{#each normalized as item, index (item.id ?? `${item.name}-${index}`)}
  <span class="agent-attachment">
    <button
      class="agent-attachment-open"
      type="button"
      aria-label={item.dataUrl ? `View ${item.name}` : `Reveal ${item.name} in Finder`}
      title={item.dataUrl ? `View ${item.name}` : `Reveal ${item.name} in Finder`}
      onclick={() => void activatePromptAttachment(PM, item)}
    >
      {#if item.dataUrl}
        <img src={item.dataUrl} alt={item.name} />
      {:else}
        <span class="agent-attachment-type">{(item.name?.split('.').pop() || 'file').slice(0, 5).toUpperCase()}</span>
      {/if}
      <span class="agent-attachment-name">{item.name}</span>
    </button>
    {#if removable}
      <button class="agent-attachment-remove" type="button" aria-label={`Remove ${item.name}`} title={`Remove ${item.name}`} onclick={() => onRemove?.(item.id)}>
        <Icon {PM} name="x" />
      </button>
    {/if}
  </span>
{/each}
