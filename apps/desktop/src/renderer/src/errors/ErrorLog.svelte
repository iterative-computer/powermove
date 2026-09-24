<script lang="ts">
  import type { Snippet } from 'svelte';
  /* Diagnostics are for reporting, not reading: a quiet toggle at the end of
     the action row, opening onto a log well whose copy control is always in
     reach. Every agent error surface shares this one shape. */
  let { details, actions }: { details: string; actions?: Snippet } = $props();
  let open = $state(false);
  let copied = $state<'idle' | 'done' | 'failed'>('idle');
  const logId = `error-log-${Math.random().toString(36).slice(2, 9)}`;
  async function copy() {
    try {
      await navigator.clipboard.writeText(details);
      copied = 'done';
    } catch {
      copied = 'failed';
    }
    setTimeout(() => { copied = 'idle'; }, 1600);
  }
</script>

{#if actions || details}
  <div class="error-actions">
    {#if actions}{@render actions()}{/if}
    {#if details}
      <button type="button" class="log-toggle" aria-expanded={open} aria-controls={logId} onclick={() => { open = !open; }}>
        Details<svg class:open viewBox="0 0 16 16" aria-hidden="true"><path d="m4.5 6.5 3.5 3.5 3.5-3.5" /></svg>
      </button>
    {/if}
  </div>
{/if}
{#if details && open}
  <div class="log" id={logId}>
    <pre>{details}</pre>
    <button type="button" class="log-copy" onclick={copy}
      aria-label={copied === 'done' ? 'Copied' : 'Copy details'} title={copied === 'failed' ? 'Select the text to copy' : 'Copy details'}>
      {#if copied === 'done'}<svg viewBox="0 0 16 16" aria-hidden="true"><path d="m3.5 8.5 3 3 6-7" /></svg>
      {:else}<svg viewBox="0 0 16 16" aria-hidden="true"><rect x="5.5" y="5.5" width="8" height="8" rx="1.5" /><path d="M10.5 3.5v-.5A1.5 1.5 0 0 0 9 1.5H4A1.5 1.5 0 0 0 2.5 3v5A1.5 1.5 0 0 0 4 9.5h.5" /></svg>{/if}
    </button>
  </div>
{/if}

<style>
  .error-actions{display:flex;flex-wrap:wrap;align-items:center;gap:8px;margin-top:12px}
  .log-toggle{display:inline-flex;align-items:center;gap:3px;margin-left:auto;height:24px;padding:0 4px 0 8px;border-radius:5px;
    color:var(--tx-3);font:inherit;font-size:var(--fs-xs);font-weight:var(--fw-medium)}
  .log-toggle:hover,.log-toggle[aria-expanded="true"]{color:var(--tx);background:var(--ink-1)}
  svg{width:14px;height:14px;flex:none;fill:none;stroke:currentColor;stroke-width:1.5;stroke-linecap:round;stroke-linejoin:round}
  .log-toggle svg{transition:transform 180ms var(--ease,ease)}
  .log-toggle svg.open{transform:rotate(180deg)}
  .log{position:relative;margin-top:8px;border-radius:8px;background:var(--ink-1)}
  pre{margin:0;max-height:160px;overflow:auto;padding:9px 38px 9px 10px;white-space:pre-wrap;overflow-wrap:anywhere;user-select:text;
    font:11px/1.55 var(--f-mono,monospace);color:var(--tx-2)}
  .log-copy{position:absolute;top:5px;right:5px;display:grid;place-items:center;width:24px;height:24px;border-radius:5px;color:var(--tx-3)}
  .log-copy:hover{background:var(--ink-2);color:var(--tx)}
  button:focus-visible{outline:2px solid var(--accent);outline-offset:2px}
  @media (prefers-reduced-motion: reduce){ .log-toggle svg{transition:none} }
</style>
