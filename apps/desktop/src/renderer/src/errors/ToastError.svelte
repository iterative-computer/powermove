<script lang="ts">
  import type { Snippet } from 'svelte';
  import Icon from '../panels/Icon.svelte';
  import ErrorFix from './ErrorFix.svelte';
  import ErrorLog from './ErrorLog.svelte';
  import { presentError } from './presentation';

  /* The toast stack is one family of objects. An error keeps the status
     toast's icon gutter, padding and type scale, and earns only the extra
     lines it needs: an explanation, any fix, the diagnostics, and recovery.
     ErrorNotice remains the panel-shaped variant for dialogs and transcripts. */
  let { PM, error, actions }: { PM: Record<string, any>; error: unknown; actions?: Snippet } = $props();
  const presentation = $derived(presentError(error));
  const diagnostics = $derived(presentation.details !== presentation.message ? presentation.details : '');
</script>

<span class="toast-icon"><Icon {PM} name="warning" /></span>
<div class="body">
  <strong>{presentation.title}</strong>
  <span class="message">{presentation.message}</span>
  <ErrorFix {presentation} />
  <div class="foot"><ErrorLog details={diagnostics} {actions} /></div>
</div>

<style>
  /* The gutter is 16px + the toast's 10px gap, so every wrapped line and every
     affordance below the title sits on one text column. */
  .body{display:flex;flex-direction:column;gap:4px;min-width:0;padding:1px 0}
  strong{font-weight:var(--fw-semibold);font-size:var(--fs-sm);line-height:16px;color:var(--tx)}
  .message{color:var(--tx-2);font-weight:var(--fw-regular);line-height:1.45;overflow-wrap:anywhere}
  .foot:empty{display:none}
  .foot :global(.error-actions){margin-top:6px}
</style>
