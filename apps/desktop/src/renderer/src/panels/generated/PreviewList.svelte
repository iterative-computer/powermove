<script lang="ts">
  import type { PreviewModel } from '../../core/generated-bindings';

  let { preview }: { preview: PreviewModel | null } = $props();
</script>

<div
  class="generated-tool-preview"
  class:error={!!preview && !preview.ok}
  role="status"
  aria-live="polite"
  aria-atomic="true"
  hidden={!preview}
>
  {#if preview}
    <strong>{preview.title}</strong>
    <p>{preview.message}</p>
    {#if preview.ok && preview.items.length}
      <ol>
        {#each preview.items as item, index (`${item.name ?? item.description ?? 'change'}-${index}`)}
          <li class:more={item.more}>
            <span>{item.description ?? item.name}</span>
            {#if item.difference}<code>{item.difference}</code>{/if}
          </li>
        {/each}
      </ol>
    {/if}
  {/if}
</div>
