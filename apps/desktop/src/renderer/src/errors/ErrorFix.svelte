<script lang="ts">
  import { bridge } from '../kernel/bridge';
  import type { ErrorPresentation } from './presentation';
  /* The part of an error someone acts on: the facts the raw text buried
     mid-sentence, and — when Powermove can fix it itself — the fix. */
  let { presentation, onupdated }: {
    presentation: ErrorPresentation;
    onupdated?: (version: string) => void;
  } = $props();

  const names = { claude: 'Claude Code', codex: 'Codex' } as const;
  const api = bridge()?.agentRuntime;
  let phase = $state<'idle' | 'updating' | 'done' | 'failed'>('idle');
  let version = $state('');
  let failure = $state('');

  const facts = $derived((presentation.facts ?? []).map((fact) =>
    phase === 'done' && fact.label === 'Installed' ? { ...fact, value: version } : fact));

  async function update(): Promise<void> {
    if (!presentation.update || !api || phase === 'updating') return;
    phase = 'updating';
    failure = '';
    try {
      ({ version } = await api.update(presentation.update));
      phase = 'done';
      onupdated?.(version);
    } catch (error) {
      failure = (error instanceof Error ? error.message : String(error))
        .replace(/^Error invoking remote method '[^']+':\s*(?:Error:\s*)?/i, '');
      phase = 'failed';
    }
  }
</script>

{#if facts.length || (presentation.update && api)}
  <div class="error-fix">
    {#if facts.length}
      <dl>
        {#each facts as fact (fact.label)}
          <div class:is-fixed={phase === 'done' && fact.label === 'Installed'}><dt>{fact.label}</dt><dd>{fact.value}</dd></div>
        {/each}
      </dl>
    {/if}
    {#if presentation.update && api}
      {@const name = names[presentation.update]}
      <div class="update" role="status" aria-live="polite">
        {#if phase === 'done'}
          <p class="done"><svg viewBox="0 0 16 16" aria-hidden="true"><path d="m3.5 8.5 3 3 6-7" /></svg>{name} {version} is installed.{onupdated ? ' Reconnecting…' : ' Try again to continue.'}</p>
        {:else}
          <button type="button" class="update-button" disabled={phase === 'updating'} onclick={update}>
            {#if phase === 'updating'}<span class="spin" aria-hidden="true"></span>{presentation.updateLabel ? `Installing ${name}…` : `Installing the latest ${name}…`}
            {:else}<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M8 2.5v8m-3.5-3.5L8 10.5 11.5 7M3 13.5h10" /></svg>{phase === 'failed' ? 'Try the fix again' : presentation.updateLabel ?? `Install latest ${name}`}{/if}
          </button>
          {#if phase === 'failed'}<p class="failed">{failure || 'The update couldn’t finish.'}</p>{/if}
        {/if}
      </div>
    {/if}
  </div>
{/if}

<style>
  .error-fix{display:flex;flex-direction:column;gap:10px;margin-top:12px}
  dl{display:grid;gap:1px;margin:0;border-radius:8px;overflow:hidden;background:var(--ink-1)}
  dl div{display:flex;justify-content:space-between;gap:12px;padding:7px 10px;background:var(--bg-field)}
  dt{color:var(--tx-3);font-size:var(--fs-xs)}
  dd{margin:0;color:var(--tx);font:var(--fs-xs)/1.5 var(--f-mono,monospace);font-variant-numeric:tabular-nums;text-align:right}
  dl div:first-child dd{color:var(--tx-2)}
  dl div.is-fixed dd{color:var(--success,var(--tx))}
  .update{display:flex;flex-direction:column;gap:6px}
  .update-button{display:flex;align-items:center;justify-content:center;gap:7px;width:100%;min-height:32px;padding:6px 10px;border-radius:7px;
    background:var(--accent);color:var(--on-accent,#fff);font:inherit;font-size:var(--fs-sm);font-weight:var(--fw-medium);line-height:1.3}
  .update-button:hover{background:var(--accent-hover)}
  .update-button:active{transform:translateY(1px)}
  .update-button:disabled{background:var(--ink-2);color:var(--tx-2);transform:none}
  .update-button:focus-visible{outline:2px solid var(--accent);outline-offset:2px}
  svg{width:14px;height:14px;flex:none;fill:none;stroke:currentColor;stroke-width:1.6;stroke-linecap:round;stroke-linejoin:round}
  .spin{flex:none;width:10px;height:10px;border:1.5px solid var(--tx-4);border-top-color:var(--tx);border-radius:50%;animation:fix-spin 1s linear infinite}
  p{margin:0;font-size:var(--fs-xs);line-height:1.5}
  .done{display:flex;align-items:center;gap:6px;color:var(--tx-2)}
  .done svg{color:var(--success,var(--accent))}
  .failed{color:var(--tx-3);overflow-wrap:anywhere}
  @keyframes fix-spin{to{transform:rotate(360deg)}}
  @media (prefers-reduced-motion: reduce){ .spin{animation:none} }
</style>
