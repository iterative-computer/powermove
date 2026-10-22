<script lang="ts">
  import { onMount } from 'svelte';
  import { etaLabel, overallProgress, secondsLeft, taskState, type TaskEntry } from './tasks.svelte';

  /* Saving and importing live here, beside the agent button: a small pill while
     work runs, and a popover with the detail when it is clicked. */
  let { PM }: { PM: Record<string, any> } = $props();
  let open = $state(false);
  let now = $state(Date.now());
  let wrap = $state<HTMLDivElement>();

  const tasks = $derived(taskState.list);
  const running = $derived(tasks.filter(task => !task.completed));
  const allDone = $derived(tasks.length > 0 && running.length === 0);
  const overall = $derived(overallProgress(tasks));
  const RING = 37.7;

  $effect(() => {
    if (!running.length) return;
    now = Date.now();
    const timer = setInterval(() => { now = Date.now(); }, 1000);
    return () => clearInterval(timer);
  });
  $effect(() => { if (!tasks.length) open = false; });

  const pillLabel = $derived(allDone ? 'Done' : overall == null ? '' : `${Math.round(overall * 100)}%`);
  const ariaLabel = $derived(allDone
    ? 'Tasks finished'
    : running.length === 1 ? `${running[0]!.title}${overall == null ? '' : `, ${Math.round(overall * 100)}%`}` : `${running.length} tasks running`);

  function percent(task: TaskEntry): string {
    return task.progress == null ? '' : `${Math.round(task.progress * 100)}%`;
  }
  /** How much of segment `index` is filled when a job runs in equal steps. */
  function segment(task: TaskEntry, index: number): number {
    if (task.progress == null || !task.step) return 0;
    return Math.min(1, Math.max(0, task.progress * task.step.total - index));
  }
  function meta(task: TaskEntry): string {
    const parts: string[] = [];
    if (task.count) parts.push(`${task.count.done}/${task.count.total}`);
    else if (task.step) parts.push(`${task.step.index}/${task.step.total}`);
    const eta = etaLabel(secondsLeft(task, now));
    if (eta) parts.push(eta);
    return parts.join(' · ');
  }

  function outside(event: PointerEvent): void {
    if (open && wrap && !wrap.contains(event.target as Node)) open = false;
  }
  function key(event: KeyboardEvent): void {
    if (open && event.key === 'Escape') { open = false; (wrap?.querySelector('button') as HTMLElement | null)?.focus(); }
  }
  onMount(() => {
    document.addEventListener('pointerdown', outside, true);
    document.addEventListener('keydown', key);
    return () => { document.removeEventListener('pointerdown', outside, true); document.removeEventListener('keydown', key); };
  });
</script>

{#if tasks.length}
  <div class="task-wrap" bind:this={wrap}>
    <button
      type="button"
      class="task-pill"
      class:on={open}
      class:done={allDone}
      aria-label={ariaLabel}
      aria-haspopup="true"
      aria-expanded={open}
      onclick={() => { open = !open; }}
    >
      {#if allDone}
        <svg class="task-mark" viewBox="0 0 16 16" aria-hidden="true"><path class="task-check" d="m4.5 8.4 2.3 2.3 4.7-5" /></svg>
      {:else}
        <svg class="task-mark" class:spin={overall == null} viewBox="0 0 16 16" aria-hidden="true">
          <circle class="task-ring-track" cx="8" cy="8" r="6" />
          <circle class="task-ring" cx="8" cy="8" r="6" stroke-dasharray={overall == null ? `${RING * .25} ${RING}` : `${overall * RING} ${RING}`} transform="rotate(-90 8 8)" />
        </svg>
      {/if}
      {#if pillLabel}<span class="task-pct">{pillLabel}</span>{/if}
    </button>

    {#if open}
      <div class="task-pop" role="group" aria-label="Running tasks">
        {#each tasks as task, i (task.key)}
          {#if i > 0}<div class="task-rule"></div>{/if}
          <div class="task-row" class:done={task.completed}>
            <div class="task-head">
              <span class="task-title">{task.title}</span>
              {#if task.completed}
                <svg class="task-done" viewBox="0 0 16 16" aria-hidden="true"><path d="m4.5 8.4 2.3 2.3 4.7-5" /></svg>
              {:else}<span class="task-num">{percent(task)}</span>{/if}
            </div>
            {#if !task.completed}
              {#if task.step}
                <div class="task-bar segments" role="progressbar" aria-label={task.title} aria-valuemin="0" aria-valuemax="100" aria-valuenow={task.progress == null ? undefined : Math.round(task.progress * 100)}>
                  {#each Array.from({ length: task.step.total }) as _, index}
                    <span class="seg"><span class="fill" style:width="{segment(task, index) * 100}%"></span></span>
                  {/each}
                </div>
              {:else}
                <div class="task-bar" role="progressbar" aria-label={task.title} aria-valuemin="0" aria-valuemax="100" aria-valuenow={task.progress == null ? undefined : Math.round(task.progress * 100)}>
                  <span class="fill" class:indeterminate={task.progress == null} style:width={task.progress == null ? undefined : `${task.progress * 100}%`}></span>
                </div>
              {/if}
              <div class="task-foot">
                <span class="task-detail" class:fade={!!task.count && task.progress != null} style:--p="{Math.round((task.progress ?? 0) * 100)}%" style:--q="{Math.min(100, Math.round((task.progress ?? 0) * 100) + 35)}%">{task.detail ?? ''}</span>
                <span class="task-meta">{meta(task)}</span>
                {#if task.action}<button type="button" class="task-action" onclick={() => task.action?.run()}>{task.action.label}</button>{/if}
              </div>
            {/if}
          </div>
        {/each}
      </div>
    {/if}
  </div>
{/if}

<style>
  .task-wrap { position: relative; flex: none; margin-right: 6px; -webkit-app-region: no-drag; }
  .task-pill {
    display: flex; align-items: center; gap: 7px; height: 28px; padding: 0 10px 0 8px; box-sizing: border-box;
    border: 1px solid var(--line); border-radius: var(--r-sm); background: var(--ink-1); color: var(--tx);
    font: inherit; font-size: var(--fs-sm); cursor: default;
    transition: background var(--dur-1) var(--ease), border-color var(--dur-1) var(--ease);
  }
  .task-pill:hover, .task-pill.on { background: var(--ink-2); }
  .task-pill.on { border-color: color-mix(in srgb, var(--accent) 55%, transparent); }
  .task-pill.done { border-color: color-mix(in srgb, var(--success) 45%, transparent); }
  .task-pill:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
  .task-mark { width: 14px; height: 14px; flex: none; fill: none; stroke-width: 2; stroke-linecap: round; stroke-linejoin: round; }
  .task-ring-track { stroke: var(--tx-4); opacity: .5; }
  .task-ring { stroke: var(--accent); transition: stroke-dasharray 240ms ease; }
  .task-mark.spin { animation: task-spin 1s linear infinite; }
  .task-check { stroke: var(--success); stroke-width: 1.8; }
  .task-pct { font-family: ui-monospace, 'JetBrains Mono', monospace; font-size: var(--fs-xs); font-variant-numeric: tabular-nums; }
  .task-pill.done .task-pct { font-family: inherit; }

  .task-pop {
    position: absolute; top: calc(100% + 6px); right: 0; z-index: 60;
    display: flex; flex-direction: column; gap: 14px; width: 340px; box-sizing: border-box; padding: 16px;
    border-radius: var(--r-md); background: var(--bg-float);
    box-shadow: 0 0 0 1px var(--line), 0 16px 40px -12px rgb(0 0 0 / .5), 0 2px 8px rgb(0 0 0 / .12);
    animation: task-in 180ms cubic-bezier(0.16, 1, 0.3, 1);
  }
  .task-rule { height: 1px; background: var(--line); }
  .task-row { display: flex; flex-direction: column; gap: 8px; }
  .task-head { display: flex; align-items: center; justify-content: space-between; gap: 12px; }
  .task-title { min-width: 0; overflow: hidden; white-space: nowrap; text-overflow: ellipsis; font-size: var(--fs-sm); font-weight: 600; color: var(--tx); }
  .task-num { flex: none; font-family: ui-monospace, 'JetBrains Mono', monospace; font-size: var(--fs-xs); color: var(--accent); font-variant-numeric: tabular-nums; }
  .task-done { flex: none; width: 14px; height: 14px; fill: none; stroke: var(--success); stroke-width: 1.8; stroke-linecap: round; stroke-linejoin: round; }

  .task-bar { display: flex; gap: 3px; height: 4px; border-radius: 2px; background: color-mix(in srgb, var(--tx) 8%, transparent); overflow: hidden; position: relative; }
  .task-bar.segments { background: none; overflow: visible; }
  .seg { flex: 1; height: 4px; border-radius: 2px; background: color-mix(in srgb, var(--tx) 8%, transparent); overflow: hidden; }
  .fill {
    display: block; height: 4px; border-radius: 2px; transition: width 240ms ease;
    background: linear-gradient(180deg, color-mix(in oklab, var(--accent), white 18%), color-mix(in oklab, var(--accent), black 14%));
  }
  .fill.indeterminate { width: 35%; animation: task-sweep 1.2s ease-in-out infinite; }

  .task-foot { display: flex; align-items: center; justify-content: space-between; gap: 12px; }
  .task-detail, .task-meta { font-family: ui-monospace, 'JetBrains Mono', monospace; font-size: var(--fs-xs); color: var(--tx-3); white-space: nowrap; }
  .task-detail { min-width: 0; overflow: hidden; text-overflow: ellipsis; }
  .task-detail.fade {
    background: linear-gradient(90deg, var(--accent) var(--p), var(--tx-3) var(--q));
    -webkit-background-clip: text; background-clip: text; color: transparent;
  }
  .task-action { flex: none; border: 0; border-radius: var(--r-sm); background: none; color: var(--tx); padding: 3px 6px; font: inherit; font-size: var(--fs-xs); }
  .task-action:hover { background: var(--bg-hover); }
  .task-action:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
  .task-meta { flex: none; font-variant-numeric: tabular-nums; }

  @keyframes task-spin { to { rotate: 360deg; } }
  @keyframes task-in { from { opacity: 0; translate: 0 -4px; } }
  @keyframes task-sweep { from { translate: -100% 0; } to { translate: 290% 0; } }
  @media (prefers-reduced-motion: reduce) {
    .task-mark.spin, .task-pop { animation: none; }
    .fill.indeterminate { animation: none; translate: 90% 0; }
    .fill, .task-ring { transition: none; }
  }
</style>
