<script lang="ts">
  import { untrack } from 'svelte';
  import SandboxCheckStatus from './SandboxCheckStatus.svelte';
  import { SANDBOX_UNAVAILABLE, type SandboxCheckReport, type SandboxCheckState } from './sandbox-check';

  /* "Test in Sandbox…" from the Library: the publish sheet's check on its
     own, so an extension can be tried as others will run it before publishing. */
  let { name, check, onclose }: {
    name: string;
    check: () => Promise<SandboxCheckReport>;
    onclose: () => void;
  } = $props();

  let state = $state<SandboxCheckState>({ status: 'running' });
  let run = 0;
  async function start(): Promise<void> {
    const current = ++run;
    state = { status: 'running' };
    let next: SandboxCheckState;
    try {
      next = { status: 'done', report: await check() };
    } catch {
      next = { status: 'error', message: SANDBOX_UNAVAILABLE };
    }
    if (current === run) state = next;
  }
  $effect(() => { untrack(() => void start()); });
</script>

<!-- svelte-ignore a11y_autofocus -->
<div class="pub-sheet" tabindex="-1" autofocus>
  <header class="acct-head">
    <h2>Sandbox compatibility check: {name}</h2>
    <p>Runs a short check with the permissions in its manifest to find compatibility issues before publishing.</p>
  </header>
  <div class="pub-form">
    <SandboxCheckStatus {state} onretry={() => void start()} />
  </div>
  <footer class="pub-foot">
    <span></span>
    <button class="btn pri" type="button" onclick={onclose}>Done</button>
  </footer>
</div>
