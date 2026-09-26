<script lang="ts">
  import { SANDBOX_OK, SANDBOX_SKIPPED, sandboxCheckLines, type SandboxCheckState } from './sandbox-check';

  /* The sandbox check as rows on the sheet's tinted card: a spinner while it
     runs, one quiet line when it passes (or is skipped for full access), and
     one line per problem when it fails. */
  let { state, onretry }: { state: SandboxCheckState; onretry?: () => void } = $props();

  const lines = $derived(state.status === 'done' ? sandboxCheckLines(state.report) : []);
</script>

<div class="sg-group pub-group pub-sandbox" aria-live="polite" aria-busy={state.status === 'running'}>
  {#if state.status === 'running'}
    <div class="settings-row pub-row">
      <span class="settings-copy pub-check is-running">
        <svg class="acct-spinner" viewBox="0 0 16 16" aria-hidden="true">
          {#each { length: 12 } as _, i}
            <line x1="8" y1="1.75" x2="8" y2="4.5" transform={`rotate(${i * 30} 8 8)`} opacity={0.16 + 0.84 * ((i + 1) / 12)} />
          {/each}
        </svg>
        <span>Checking how it runs in the sandbox…</span>
      </span>
    </div>
  {:else if state.status === 'error'}
    <div class="settings-row pub-row">
      <span class="settings-copy"><span class="pub-problem">{state.message}</span></span>
      {#if onretry}<button class="btn" type="button" onclick={onretry}>Check Again</button>{/if}
    </div>
  {:else if state.report.skipped}
    <div class="settings-row pub-row">
      <span class="settings-copy pub-check" data-sandbox="skipped"><span>{SANDBOX_SKIPPED}</span></span>
    </div>
  {:else if state.report.ok}
    <div class="settings-row pub-row">
      <span class="settings-copy pub-check is-ok" data-sandbox="ok">
        <svg class="pub-check-mark" viewBox="0 0 16 16" aria-hidden="true"><path d="M3.5 8.5l3 3 6-7" /></svg>
        <span>{SANDBOX_OK}</span>
      </span>
    </div>
  {:else}
    {#each lines as line, index (index)}
      <div class="settings-row pub-row" data-sandbox="problem">
        <span class="settings-copy"><span class="pub-problem pub-sandbox-line">{line}</span></span>
        {#if onretry && index === 0}<button class="btn" type="button" onclick={onretry}>Check Again</button>{/if}
      </div>
    {/each}
  {/if}
</div>
