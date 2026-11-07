<script lang="ts">
  import { SANDBOX_OK, SANDBOX_SKIPPED, sandboxAdviceLines, sandboxCheckLines, type SandboxCheckState } from './sandbox-check';

  /* The sandbox check as Settings rows: a title and a line under it, the
     state at the right edge (a spinner, a tick, or Check Again). A failure
     adds one row per problem, the exact reason as its text, and offers the
     repair agent beside Check Again. */
  let { state, onretry, onfix, fixing = false, disabled = false }: {
    state: SandboxCheckState; onretry?: () => void; onfix?: () => void; fixing?: boolean; disabled?: boolean;
  } = $props();

  const lines = $derived(state.status === 'done' ? sandboxCheckLines(state.report) : []);
  /* Effect validation hints never block publishing; they follow the result
     as advice, amber rather than red, with the exact fix to copy. */
  const advice = $derived(state.status === 'done' ? sandboxAdviceLines(state.report) : []);
</script>

<div class="sg-group pub-group pub-sandbox" aria-live="polite" aria-busy={state.status === 'running'}>
  {#if state.status === 'running'}
    <div class="settings-row pub-row">
      <span class="settings-copy"><b>Checking compatibility</b><span>Running it in the sandbox with the permissions it declares.</span></span>
      <svg class="acct-spinner pub-state" viewBox="0 0 16 16" aria-hidden="true">
        {#each { length: 12 } as _, i}
          <line x1="8" y1="1.75" x2="8" y2="4.5" transform={`rotate(${i * 30} 8 8)`} opacity={0.16 + 0.84 * ((i + 1) / 12)} />
        {/each}
      </svg>
    </div>
  {:else if state.status === 'error'}
    <div class="settings-row pub-row">
      <span class="settings-copy"><b>Couldn’t check</b><span class="pub-problem">{state.message}</span></span>
      {#if onretry}<button class="btn" type="button" onclick={onretry}>Check Again</button>{/if}
    </div>
  {:else if state.report.skipped}
    <div class="settings-row pub-row" data-sandbox="skipped">
      <span class="settings-copy"><b>Full access</b><span>{SANDBOX_SKIPPED}</span></span>
    </div>
  {:else if state.report.ok}
    <div class="settings-row pub-row" data-sandbox="ok">
      <span class="settings-copy"><b>{SANDBOX_OK}</b><span>It runs with only the permissions it declares.</span></span>
      <svg class="pub-check-mark pub-state" viewBox="0 0 16 16" aria-hidden="true"><path d="M3.5 8.5l3 3 6-7" /></svg>
    </div>
  {:else}
    <div class="settings-row pub-row" data-sandbox="failed">
      <span class="settings-copy"><b>Not compatible with the sandbox</b><span>{onfix ? 'The agent can fix this for you, or fix it yourself and check again.' : `Fix ${lines.length === 1 ? 'this' : 'these'}, then check again.`}</span></span>
      {#if onretry || onfix}
        <span class="pub-sandbox-actions">
          {#if onretry}<button class="btn" type="button" disabled={disabled || fixing} onclick={onretry}>Check Again</button>{/if}
          {#if onfix}<button class="btn pri" type="button" disabled={disabled || fixing} onclick={onfix}>{fixing ? 'Opening Agent…' : 'Fix with Agent'}</button>{/if}
        </span>
      {/if}
    </div>
    <ul class="pub-sandbox-problems">
      {#each lines as line, index (index)}
        <li class="pub-sandbox-line" data-sandbox="problem">{line}</li>
      {/each}
    </ul>
  {/if}
  {#if advice.length}
    <div class="settings-row pub-row pub-advice-head" data-sandbox="advice">
      <span class="settings-copy"><b>{advice.length === 1 ? 'One suggestion' : `${advice.length} suggestions`}</b><span>They don’t block publishing.</span></span>
    </div>
    <ul class="pub-sandbox-problems pub-advice">
      {#each advice as item, index (index)}
        <li class="pub-sandbox-line pub-advice-line" data-sandbox="effect-warning">
          <span class="pub-advice-title">{item.title}.</span> {item.message}
          <span class="pub-advice-fix">{item.fix}</span>
        </li>
      {/each}
    </ul>
  {/if}
</div>
