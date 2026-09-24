<script lang="ts">
  import type { Snippet } from 'svelte';
  import ErrorFix from './ErrorFix.svelte';
  import ErrorLog from './ErrorLog.svelte';
  import { presentError, type NoticeKind } from './presentation';
  /* One shape for both: an alert is the same notice with a softer marker and
     no diagnostics, because the agent's own account is the whole story. */
  let { error, live = true, kind = 'error', actions, fallbackTitle, onupdated }: {
    error: unknown; live?: boolean; kind?: Exclude<NoticeKind, 'plain'>; actions?: Snippet;
    /** Names the failure when the text isn't one presentError recognises. */
    fallbackTitle?: string;
    onupdated?: (version: string) => void;
  } = $props();
  const presented = $derived(presentError(error));
  const presentation = $derived(fallbackTitle && presented.title === 'Something needs attention'
    ? { ...presented, title: fallbackTitle } : presented);
  const alert = $derived(kind === 'alert');
  // When nothing was translated the message already is the raw text.
  const diagnostics = $derived(!alert && presentation.details !== presentation.message ? presentation.details : '');
</script>

<div class="error-notice" class:is-alert={alert} role={live && !alert ? 'alert' : undefined}>
  {#if alert}
    <p class="alert-text"><span class="error-marker" aria-hidden="true">!</span>{presentation.details || presentation.message}</p>
  {:else}
    <div class="error-heading"><span class="error-marker" aria-hidden="true">!</span><strong>{presentation.title}</strong></div>
    <div class="error-body">
      <p>{presentation.message}</p>
      <ErrorFix {presentation} {onupdated} />
    </div>
  {/if}
  <div class:error-body={!alert}><ErrorLog details={diagnostics} {actions} /></div>
</div>

<style>
  .error-notice{min-width:0;width:100%;padding:12px;border:0;border-radius:10px;background:color-mix(in srgb,var(--danger) 4%,var(--bg-float));color:var(--tx);font:var(--fs-sm)/1.5 var(--f-ui);box-sizing:border-box;overflow-wrap:anywhere}
  .error-heading{display:flex;align-items:center;gap:8px;margin-bottom:4px}
  /* Everything under the title hangs from one text column beside the marker. */
  .error-body{padding-left:24px}
  strong{font-weight:600;font-size:var(--fs-sm)}
  .error-marker{display:grid;place-items:center;flex:none;width:16px;height:16px;border-radius:50%;background:color-mix(in srgb,var(--danger) 13%,transparent);color:var(--danger);font-size:11px;font-weight:700}
  /* An alert says its piece on one line, in the app's warning tint. */
  .error-notice.is-alert{background:color-mix(in srgb,var(--warning) 5%,var(--bg-float))}
  .error-notice.is-alert .error-marker{background:color-mix(in srgb,var(--warning) 15%,transparent);color:var(--warning);margin-top:1px}
  .alert-text{display:flex;align-items:flex-start;gap:8px;margin:0;color:var(--tx)!important;font:inherit}
  .error-notice p{margin:0;color:var(--tx-2)!important;font:inherit}
</style>
