<script lang="ts">
  import { tick, type Snippet } from 'svelte';
  import { inspectorContext } from './context';
  import { inspectorRefresh } from './refresh.svelte.js';

  /* Shared body for the Properties and Layer Effects panels: one surface owns
     the row, chip and stopwatch styling so both panels stay identical. */
  let { panelId, children, scrollKey }: { panelId: string; children: Snippet; scrollKey?: string } = $props();
  const { doc, syncTime } = inspectorContext();
  const scrollProject = $derived(doc.proj);
  let surface = $state<HTMLDivElement>();
  const positions = new Map<string, number>();
  let previousProject: typeof doc.proj | undefined;
  let previousKey: string | undefined;
  let restoring = false;

  $effect.pre(() => {
    const key = scrollKey;
    const project = scrollProject;
    const body = surface?.closest<HTMLElement>('.body');
    if (key === undefined || !body) return;
    if (project === previousProject && key === previousKey) return;

    // Save before shorter incoming content can clamp the outgoing scroll.
    if (project !== previousProject) positions.clear();
    else if (previousKey !== undefined && !restoring) positions.set(previousKey, body.scrollTop);
    previousProject = project;
    previousKey = key;
    restoring = true;
    let cancelled = false;
    void tick().then(() => {
      if (cancelled) return;
      body.scrollTop = positions.get(key) ?? 0;
      restoring = false;
    });
    return () => { cancelled = true; };
  });
</script>

<div bind:this={surface} class="insp" data-svelte-panel={panelId} data-inspector-refresh={inspectorRefresh.version}
  onpointerdowncapture={syncTime} onkeydowncapture={syncTime} onwheelcapture={syncTime}>
  {@render children()}
</div>

<style>
  .insp {
    display: flex;
    flex-direction: column;
    gap: 0;
    padding: 0 var(--pad) 24px;
  }

  .insp :global(.row.split .k) {
    width: 90px;
    font-size: 11px;
    line-height: 1.2;
  }
  .insp :global(.row.split) { gap: 6px; }
  /* Tall rows (the text field) keep their label and stopwatch on the same
     28px band as every single-line row, so the diamond and the title stay
     centered on each other while the field grows below. */
  .insp :global(.row.split:has(textarea)) { height: auto; min-height: 54px; align-items: start; }
  .insp :global(.row.split:has(textarea) > .k) { height: 28px; }
  .insp :global(.row.split:has(textarea) > .stopwatch) { margin-top: 5px; }
  /* Reserve the diamond's 18px + 6px gap even on non-animated rows.
     Border-box padding keeps both the text and control columns aligned. */
  .insp :global(.row.split:not(:has(> .stopwatch)) > .k) { width: 114px; padding-left: 24px; }
  .insp :global(.property-stopwatch) { width: 18px; height: 24px; }

  /* Panel actions use the same flat material as property fields. */
  .insp :global(.chip) {
    min-width: 0;
    max-width: 100%;
    height: var(--ctl-h);
    background: var(--bg-field);
    box-shadow: none;
    white-space: normal;
  }

  .insp :global(.chip:hover) { background: var(--bg-row-hi); color: var(--tx); }
  .insp :global(.chip:active) { background: var(--bg-row); transform: none; }
  .insp :global(.chip:focus-visible) { outline: 2px solid var(--accent); outline-offset: 2px; }
  .insp :global(.chip:disabled) { opacity: var(--disabled); pointer-events: none; }
  .insp :global(.chip.ghost) { background: transparent; }
  .insp :global(.chip.ghost:hover) { background: var(--ink-1); }
  .insp :global(.chip.solid) { background: var(--accent); color: var(--on-accent); }
  .insp :global(.chip.solid:hover) { background: var(--accent-hover); }

  .insp :global(.chip.wide) {
    width: 100%;
    margin: 0 0 4px;
  }

  .insp :global(.stopwatch) {
    display: grid;
    flex: none;
    width: 18px;
    height: 18px;
    place-items: center;
    border-radius: var(--r-xs);
    color: var(--tx-4);
    transition: background var(--dur-1), color var(--dur-1);
  }

  .insp :global(.stopwatch:hover) {
    background: var(--ink-1);
    color: var(--tx-2);
  }

  .insp :global(.stopwatch.on) {
    color: var(--accent);
  }

  .insp :global(.stopwatch svg) {
    width: 11px;
    height: 11px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.8;
  }

  .insp :global(.property-stopwatch.at-key svg) { fill: currentColor; }

  .insp :global(.stopwatch.property-stopwatch svg) { width: 14px; height: 14px; stroke-width: 1.5; }

  .insp :global(.kf i) { width: 5px; height: 5px; }

  .insp :global(.grp) {
    margin-left: 0;
  }

  .insp :global(.grp .row) {
    margin-left: 12px;
  }

  .insp :global(.twirl) {
    display: grid;
    flex: none;
    width: 14px;
    height: 14px;
    place-items: center;
    color: var(--tx-3);
    transition: transform var(--dur-2) var(--ease-io);
  }

  .insp :global(.twirl.open) {
    transform: rotate(90deg);
  }

  .insp :global(.empty) {
    padding: 28px 16px;
    color: var(--tx-3);
    font-size: var(--fs-sm);
    line-height: var(--lh);
    text-align: center;
    text-wrap: pretty;
  }

  .insp :global(.inspector-header) {
    height: 48px;
    margin: 0 calc(-1 * var(--pad)) 8px;
    padding: 0 12px 0 16px;
    border-bottom: 0;
    border-radius: 0;
    background: transparent;
  }
  .insp :global(.inspector-header .k) { font-size: 12px; }

  .insp :global(.inspector-header:hover) {
    background: transparent;
  }

  .insp :global(.inspector-header .k) {
    color: var(--tx);
    font-weight: var(--fw-medium);
  }

  .insp :global(.inspector-header .k .sub) {
    color: var(--tx-3);
    font-weight: var(--fw-regular);
  }

  .insp :global(.inspector-header .iconbtn) {
    width: 24px;
    height: 24px;
    color: var(--tx-3);
  }
</style>
