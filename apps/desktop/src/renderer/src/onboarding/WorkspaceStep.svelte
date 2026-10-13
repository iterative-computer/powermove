<script lang="ts">
  import type { OnboardingChoice } from '../../../shared/creative-workspace';
  import SuitcaseTowers from './SuitcaseTowers.svelte';

  let { onback, onbegin }: { onback: () => void; onbegin: (choice: Pick<OnboardingChoice, 'workspaceImport'>) => Promise<void> } = $props();
  let busy = $state(false);
  let status = $state('');

  async function begin(workspaceImport: OnboardingChoice['workspaceImport']) {
    if (busy) return;
    busy = true;
    status = workspaceImport ? 'Preparing your workspace agent…' : 'Opening Powermove…';
    try { await onbegin({ workspaceImport }); }
    catch { busy = false; status = 'Could not open your workspace. Try again, or start fresh.'; }
  }
</script>

<SuitcaseTowers />

<main class="workspace-step" aria-labelledby="workspace-title" aria-busy={busy}>
  <button class="back" type="button" aria-label="Back to speech model choice" disabled={busy} onclick={onback}>
    <svg viewBox="0 0 24 24" aria-hidden="true"><path d="m14 6-6 6 6 6" /></svg>
  </button>
  <p class="source" aria-label="From After Effects"><span class="app-mark" aria-hidden="true">Ae</span>After Effects</p>
  <h1 id="workspace-title" tabindex="-1">Grab your suitcase.</h1>
  <p class="intro">Bring your panels and tools along.<br />Your agent will unpack them in Powermove.</p>
  <div class="actions">
    <button class="primary" type="button" disabled={busy} onclick={() => begin('after-effects')}>Bring my workspace</button>
    <button class="secondary" type="button" disabled={busy} onclick={() => begin(null)}>Start fresh</button>
  </div>
  <p class="status" role="status" aria-live="polite">{status}</p>
</main>

<style>
  .workspace-step { position: relative; z-index: 1; display: flex; flex-direction: column; align-items: center; width: min(340px, calc(100vw - 280px)); padding-top: 48px; text-align: center; -webkit-app-region: no-drag; }
  /* Same entrance as the welcome card (welcome-enter in welcome.css), staggered in as the suitcases rise. */
  .workspace-step > * { animation: welcome-enter 800ms cubic-bezier(0.16, 1, 0.3, 1) calc(240ms + var(--i, 0) * 120ms) backwards; }
  .workspace-step > h1 { --i: 1; }
  .workspace-step > .intro { --i: 2; }
  .workspace-step > .actions, .workspace-step > .status { --i: 3; }
  .back { position: absolute; top: 0; left: 0; width: 32px; height: 32px; display: grid; place-items: center; background: transparent; color: var(--tx-3); }
  .back:hover { background: var(--ink-1); color: var(--tx); }
  .source { display: flex; align-items: center; gap: 8px; margin: 0 0 12px; color: var(--tx-2); font-size: var(--fs-sm); font-weight: var(--fw-medium); }
  .app-mark { display: grid; place-items: center; width: 22px; height: 22px; border-radius: 5px; background: var(--tx); color: var(--bg-window); font-size: 12px; letter-spacing: -.04em; }
  h1 { margin: 0 0 12px; font-size: clamp(30px, 4vw, 36px); line-height: 1.1; font-weight: var(--fw-regular); letter-spacing: -.035em; white-space: nowrap; outline: none; }
  .intro { margin: 0; color: var(--tx-2); font-size: var(--fs-lg); line-height: 1.5; }
  .actions { display: flex; align-items: center; gap: 8px; margin-top: 28px; }
  svg { width: 20px; height: 20px; fill: none; stroke: currentColor; stroke-width: 1.5; stroke-linecap: round; stroke-linejoin: round; }
  button { border: 0; border-radius: var(--r-sm); font: 500 var(--fs-sm) 'Geist', sans-serif; cursor: default; transition: background var(--dur-1) var(--ease); }
  .primary, .secondary { min-height: 38px; padding: 0 18px; }
  .primary { color: var(--on-accent); background: var(--accent); }
  .primary:hover { background: var(--accent-hover); }
  .secondary { color: var(--tx-2); background: transparent; }
  .secondary:hover { color: var(--tx); background: var(--ink-1); }
  button:focus-visible { outline: 2px solid var(--accent); outline-offset: 3px; }
  button:disabled { opacity: var(--disabled); }
  .status { min-height: 18px; margin: 10px 0 0; color: var(--tx-2); font-size: var(--fs-sm); }
  @media (prefers-reduced-motion: reduce) { .workspace-step > * { animation: none; } button { transition: none; } }
</style>
