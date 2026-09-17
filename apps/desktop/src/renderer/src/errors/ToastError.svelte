<script lang="ts">
  import type { Snippet } from 'svelte';
  import Icon from '../panels/Icon.svelte';
  import { copyErrorDetails } from './copy';
  import { presentError } from './presentation';

  /* The toast stack is one family of objects. An error keeps the status
     toast's icon gutter, padding and type scale, and earns only the extra
     lines it needs: an explanation, the diagnostics, and any recovery action.
     ErrorNotice remains the panel-shaped variant for dialogs and transcripts. */
  let { PM, error, actions }: { PM: Record<string, any>; error: unknown; actions?: Snippet } = $props();
  const presentation = $derived(presentError(error));
  let open = $state(false);
  let copyLabel = $state('Copy details');
</script>

<span class="toast-icon"><Icon {PM} name="warning" /></span>
<div class="body">
  <strong>{presentation.title}</strong>
  <span class="message">{presentation.message}</span>
  {#if presentation.details}
    <button
      type="button"
      class="quiet disclosure"
      aria-expanded={open}
      onclick={() => { open = !open; copyLabel = 'Copy details'; }}
    >
      <span class="chev" class:open><Icon {PM} name="chevD" /></span>Technical details
    </button>
    {#if open}
      <pre>{presentation.details}</pre>
      <button type="button" class="quiet" onclick={async () => { copyLabel = await copyErrorDetails(presentation.details); }}>{copyLabel}</button>
    {/if}
  {/if}
  {#if actions}<div class="actions">{@render actions()}</div>{/if}
</div>

<style>
  /* The gutter is 16px + the toast's 10px gap, so every wrapped line and every
     affordance below the title sits on one text column. */
  .body{display:flex;flex-direction:column;gap:4px;min-width:0;padding:1px 0}
  strong{font-weight:var(--fw-semibold);font-size:var(--fs-sm);line-height:16px;color:var(--tx)}
  .message{color:var(--tx-2);font-weight:var(--fw-regular);line-height:1.45;overflow-wrap:anywhere}
  .quiet{align-self:flex-start;max-width:100%;height:22px;margin-top:2px;padding:0 8px;border-radius:var(--r-sm);
    background:var(--ink-1);color:var(--tx-2);font:inherit;font-size:var(--fs-xs);font-weight:var(--fw-medium)}
  .quiet:hover{background:var(--ink-2);color:var(--tx)}
  .quiet:focus-visible{outline:2px solid var(--accent);outline-offset:2px}
  .disclosure{display:flex;align-items:center;gap:4px}
  /* The app's disclosure idiom: a down chevron turned aside while collapsed. */
  .chev{display:grid;place-items:center;width:12px;height:12px;transform:rotate(-90deg);transition:transform 200ms var(--ease)}
  .chev :global(.pm-icon){width:12px;height:12px;display:block}
  .chev.open{transform:rotate(0deg)}
  pre{margin:4px 0 0;max-height:150px;overflow:auto;white-space:pre-wrap;overflow-wrap:anywhere;user-select:text;
    padding:8px;border-radius:var(--r-sm);background:var(--ink-1);color:var(--tx-2);font:var(--fs-xs)/1.5 var(--f-mono,monospace)}
  pre:focus-visible{outline:2px solid var(--accent);outline-offset:2px}
  .actions{display:flex;flex-wrap:wrap;gap:6px;margin-top:4px}
  @media (prefers-reduced-motion: reduce){ .chev{transition:none} }
</style>
