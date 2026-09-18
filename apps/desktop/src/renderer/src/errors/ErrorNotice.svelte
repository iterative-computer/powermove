<script lang="ts">
  import type { Snippet } from 'svelte';
  import { presentError } from './presentation';
  let { error, live = true, actions }: { error: unknown; live?: boolean; actions?: Snippet } = $props();
  const presentation = $derived(presentError(error));
  let copyState = $state('Copy details');
  async function copy() {
    try {
      await navigator.clipboard.writeText(presentation.details);
      copyState = 'Copied';
    } catch {
      copyState = 'Select the details to copy';
    }
  }
</script>

<div class="error-notice" role={live ? 'alert' : undefined}>
  <div class="error-heading"><span class="error-marker" aria-hidden="true">!</span><strong>{presentation.title}</strong></div>
  <p>{presentation.message}</p>
  {#if presentation.details}
    <details>
      <summary>Technical details</summary>
      <pre>{presentation.details}</pre>
      <button type="button" onclick={copy}>{copyState}</button>
    </details>
  {/if}
  {#if actions}<div class="error-actions">{@render actions()}</div>{/if}
</div>

<style>
  .error-notice{min-width:0;width:100%;padding:12px;border:0;border-radius:10px;background:color-mix(in srgb,var(--danger) 4%,var(--bg-float));color:var(--tx);font:var(--fs-sm)/1.5 var(--f-ui);box-sizing:border-box;overflow-wrap:anywhere}
  .error-actions{display:flex;flex-wrap:wrap;gap:8px;margin-top:12px}
  .error-heading{display:flex;align-items:center;gap:8px;margin-bottom:6px}
  strong{font-weight:600;font-size:var(--fs-sm)}
  .error-marker{display:grid;place-items:center;flex:none;width:16px;height:16px;border-radius:50%;background:color-mix(in srgb,var(--danger) 13%,transparent);color:var(--danger);font-size:11px;font-weight:700}
  .error-notice p{margin:0;color:var(--tx-2)!important;font:inherit}
  details{margin-top:10px;color:var(--tx-3);font-size:var(--fs-xs)}
  summary{width:fit-content;max-width:100%}
  summary:hover{color:var(--tx)}
  pre{margin:8px 0;max-height:180px;overflow:auto;white-space:pre-wrap;overflow-wrap:anywhere;user-select:text;font:11px/1.5 var(--f-mono,monospace);color:var(--tx-2)}
  button{padding:4px 7px;border-radius:5px;background:var(--ink-1);color:var(--tx-2);font:inherit;text-align:left}
  button:hover{background:var(--ink-2);color:var(--tx)}
  :is(summary,button,pre):focus-visible{outline:2px solid var(--accent);outline-offset:3px}
</style>
