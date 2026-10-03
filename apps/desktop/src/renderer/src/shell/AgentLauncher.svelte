<script lang="ts">
  import { onMount } from 'svelte';
  import Icon from '../panels/Icon.svelte';
  import { agentState } from '../panels/agent/agent-state.svelte';
  import { elapsedLabel, launcherStatus } from '../panels/agent/launcher-status';

  /* The agent's home in the titlebar. Idle it is one icon; while the agent
     works it widens just enough to say what it is doing and how many tasks
     run. Click opens the conversation; drag it onto the workspace to dock it. */
  let { PM }: { PM: Record<string, any> } = $props();
  let open = $state(false);
  let unseen = $state(false);
  let receiving = $state(false);
  let hovering = $state(false);
  // A pinned (docked) agent panel carries the conversation; the launcher steps aside.
  let docked = $state(false);
  let now = $state(Date.now());
  let wasRunning = false;
  let lastThread: string | undefined;
  let hoverTimer: ReturnType<typeof setTimeout> | undefined;

  // A run that settles while the popover is closed is news until it is read.
  $effect(() => {
    const running = agentState.phase === 'running';
    const thread = agentState.threadId;
    if (thread !== lastThread) { lastThread = thread; wasRunning = running; return; }
    if (wasRunning && !running && !open) unseen = true;
    wasRunning = running;
  });
  $effect(() => { if (open) unseen = false; });

  const threadTitle = $derived((agentState.threads ?? []).find(thread => thread.id === agentState.threadId)?.title ?? '');
  const status = $derived(launcherStatus({ ...agentState, threadTitle }, unseen));
  const live = $derived(status.tone === 'working' || status.tone === 'attention');
  $effect(() => {
    if (!live || !status.startedAt) return;
    now = Date.now();
    const timer = setInterval(() => { now = Date.now(); }, 1000);
    return () => clearInterval(timer);
  });
  const elapsed = $derived(live && status.startedAt ? elapsedLabel(now - status.startedAt) : '');
  const tasks = $derived((agentState.threads ?? []).filter(thread =>
    thread.busy || (thread.id === agentState.threadId && agentState.phase === 'running')));
  const showTasks = $derived(hovering && !open && tasks.length > 0);
  const compact = $derived(status.tone === 'idle');
  const announcement = $derived(status.tone === 'idle' ? '' : `Agent: ${status.label}`);
  const ariaLabel = $derived([
    status.tone === 'idle' ? 'Ask the agent' : `Agent: ${status.label}`,
    elapsed && `running ${elapsed}`,
    status.tasks > 1 && `${status.tasks} tasks running`
  ].filter(Boolean).join(', '));

  function toggle(): void {
    hovering = false;
    if (PM.AgentShell?.consumeDrag?.()) return;
    if (PM.AgentShell?.isOpen?.()) { PM.AgentShell.close(); return; }
    // Reopen the conversation the launcher is reporting on, not another context.
    const keepContext = status.tone !== 'idle';
    if (PM.SpatialAssistant?.open) PM.SpatialAssistant.open({ keepContext });
    else PM.AgentShell?.open?.();
  }

  function press(event: PointerEvent): void {
    PM.AgentShell?.dragToDock?.(event, event.currentTarget as HTMLElement);
  }

  function openTask(id: string): void {
    hovering = false;
    if (id !== agentState.threadId && !agentState.threadSwitchBlocked) PM.AgentUI?.switchThread?.(id);
    if (!PM.AgentShell?.isOpen?.()) PM.SpatialAssistant?.open?.({ keepContext: true });
  }

  function enter(): void { clearTimeout(hoverTimer); hoverTimer = setTimeout(() => { hovering = true; }, 180); }
  function leave(): void { clearTimeout(hoverTimer); hoverTimer = setTimeout(() => { hovering = false; }, 120); }

  onMount(() => {
    open = !!PM.AgentShell?.isOpen?.();
    const syncDocked = () => { docked = !!PM.AgentShell?.isDocked?.(); };
    syncDocked();
    const offLayout = PM.bus?.on?.('layout:applied', syncDocked);
    const offProjects = PM.bus?.on?.('projects:screen', syncDocked);
    const offPopover = PM.bus?.on?.('agent:popover', (value: boolean) => { open = !!value; });
    const offCollapse = PM.bus?.on?.('agent:collapse', () => {
      receiving = false;
      requestAnimationFrame(() => { receiving = true; });
    });
    return () => { offLayout?.(); offProjects?.(); offPopover?.(); offCollapse?.(); clearTimeout(hoverTimer); };
  });
</script>

{#if !docked}
<div class="agent-launcher-wrap" role="presentation" onpointerenter={enter} onpointerleave={leave}>
  <button
    id="agent-launcher"
    class="agent-launcher"
    class:compact
    class:on={open}
    class:receiving
    type="button"
    data-tone={status.tone}
    title={status.tone === 'idle' ? 'Ask the agent (⌘⇧K). Drag to dock.' : status.label}
    aria-label={ariaLabel}
    aria-haspopup="dialog"
    aria-controls="agent-popover"
    aria-expanded={open}
    onpointerdown={press}
    onclick={toggle}
    onanimationend={() => { receiving = false; }}
  >
    <span class="agent-launcher-mark" aria-hidden="true">
      {#if status.tone === 'working'}
        <svg class="agent-launcher-spinner" viewBox="0 0 16 16"><circle cx="8" cy="8" r="6" /><path d="M8 2a6 6 0 0 1 6 6" /></svg>
      {:else if status.tone === 'done'}
        <svg viewBox="0 0 16 16"><path class="agent-launcher-check" d="m4.5 8.4 2.3 2.3 4.7-5" /></svg>
      {:else if status.tone === 'attention' || status.tone === 'review' || status.tone === 'error'}
        <span class="agent-launcher-dot"></span>
      {:else}
        <Icon {PM} name="sparkle" />
      {/if}
    </span>
    {#if !compact}
      {#key status.label}
        <span class="agent-launcher-label">{status.label}</span>
      {/key}
      {#if elapsed}<span class="agent-launcher-meta">{elapsed}</span>{/if}
      {#if status.tasks > 1}
        <span class="agent-launcher-count" title={`${status.tasks} tasks running`}>{status.tasks}</span>
      {/if}
    {/if}
  </button>
  <span class="agent-launcher-announce" aria-live="polite">{announcement}</span>

  {#if showTasks}
    <div class="agent-launcher-tasks" role="group" aria-label="Running agent tasks">
      <p class="agent-launcher-tasks-head">{tasks.length === 1 ? 'Running' : `${tasks.length} running`}</p>
      {#each tasks as task (task.id)}
        {@const current = task.id === agentState.threadId}
        <button class="agent-launcher-task" type="button" onclick={() => openTask(task.id)}>
          <span class="agent-launcher-task-dot" aria-hidden="true"></span>
          <span class="agent-launcher-task-text">
            <span class="agent-launcher-task-title">{task.title || 'New thread'}</span>
            <span class="agent-launcher-task-step">{current ? (status.label !== task.title ? status.label : 'Working…') : 'Working in the background'}</span>
          </span>
          {#if current && elapsed}<span class="agent-launcher-meta">{elapsed}</span>{/if}
        </button>
      {/each}
    </div>
  {/if}
</div>
{/if}

<style>
  .agent-launcher-wrap { position: relative; flex: 0 1 auto; min-width: 0; margin-right: 6px; -webkit-app-region: no-drag; }
  .agent-launcher {
    position: relative; isolation: isolate;
    display: flex; align-items: center; gap: 6px;
    max-width: 200px; height: 28px; padding: 0 8px 0 7px; box-sizing: border-box;
    border: 0; border-radius: var(--r-sm);
    background: var(--glass-fill-hi); box-shadow: var(--glass-edge-hi);
    color: var(--tx); font: inherit; font-size: var(--fs-sm); text-align: start; cursor: default; touch-action: none;
    transition: background var(--dur-1) var(--ease), color var(--dur-1) var(--ease);
  }
  /* Idle: one icon on a filled square, the size of the titlebar's icon buttons. */
  .agent-launcher.compact { width: 28px; padding: 0; justify-content: center; color: var(--tx-2); }
  .agent-launcher:hover, .agent-launcher.on { background: var(--ink-2); }
  .agent-launcher:active { transform: scale(.96); }
  .agent-launcher:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
  :global(.agent-launcher.drag-src) { opacity: .5; }

  .agent-launcher-mark { display: grid; place-items: center; flex: none; width: 16px; height: 16px; color: var(--accent); }
  .agent-launcher-mark :global(.pm-icon) { width: 14px; height: 14px; fill: currentColor; }
  .agent-launcher-mark svg { width: 16px; height: 16px; fill: none; stroke: currentColor; stroke-width: 1.7; stroke-linecap: round; stroke-linejoin: round; }
  .agent-launcher-spinner circle { opacity: .22; }
  .agent-launcher-spinner { animation: agent-launcher-spin 900ms linear infinite; }
  [data-tone="done"] .agent-launcher-mark { color: var(--success); }
  .agent-launcher-check { stroke-dasharray: 12; stroke-dashoffset: 12; animation: agent-launcher-draw 360ms cubic-bezier(0.16, 1, 0.3, 1) forwards; }
  .agent-launcher-dot { width: 7px; height: 7px; border-radius: 50%; background: currentColor; }
  [data-tone="attention"] .agent-launcher-mark { color: var(--warning); }
  [data-tone="attention"] .agent-launcher-dot { animation: agent-launcher-pulse 1.4s ease-in-out infinite; }
  [data-tone="error"] .agent-launcher-mark { color: var(--danger); }

  .agent-launcher-label {
    flex: 0 1 auto; min-width: 0; max-width: 128px; overflow: hidden; white-space: nowrap; text-overflow: ellipsis;
    animation: agent-launcher-swap 260ms cubic-bezier(0.16, 1, 0.3, 1);
  }
  .agent-launcher-meta { flex: none; color: var(--tx-3); font-size: var(--fs-xs); font-variant-numeric: tabular-nums; }
  .agent-launcher-count {
    flex: none; min-width: 18px; height: 18px; padding: 0 5px; border-radius: 9px; box-sizing: border-box;
    display: grid; place-items: center; background: var(--ink-2); color: var(--tx-2);
    font-size: var(--fs-xs); font-variant-numeric: tabular-nums;
  }

  /* The popover folds into the launcher on send; the launcher takes the run. */
  .agent-launcher.receiving { animation: agent-launcher-receive 420ms cubic-bezier(0.16, 1, 0.3, 1); }

  .agent-launcher-announce { position: absolute; width: 1px; height: 1px; overflow: hidden; clip-path: inset(50%); white-space: nowrap; }

  .agent-launcher-tasks {
    position: absolute; top: calc(100% + 6px); left: 0; z-index: 60;
    width: 260px; box-sizing: border-box; padding: 4px;
    border-radius: var(--r-md); background: var(--bg-float);
    box-shadow: 0 0 0 1px var(--line), 0 10px 28px -10px rgb(0 0 0 / .3), 0 2px 8px rgb(0 0 0 / .12);
    animation: agent-launcher-tasks-in 180ms cubic-bezier(0.16, 1, 0.3, 1);
  }
  .agent-launcher-tasks-head { margin: 0; padding: 6px 8px 4px; color: var(--tx-3); font-size: var(--fs-xs); }
  .agent-launcher-task {
    display: flex; align-items: center; gap: 9px; width: 100%; padding: 6px 8px;
    border: 0; border-radius: var(--r-sm); background: none; color: var(--tx); font: inherit; text-align: start;
  }
  .agent-launcher-task:hover { background: var(--ink-1); }
  .agent-launcher-task:focus-visible { outline: 2px solid var(--accent); outline-offset: -2px; }
  .agent-launcher-task-dot { flex: none; width: 6px; height: 6px; border-radius: 50%; background: var(--accent); animation: agent-launcher-pulse 1.4s ease-in-out infinite; }
  .agent-launcher-task-text { display: grid; flex: 1; min-width: 0; }
  .agent-launcher-task-title, .agent-launcher-task-step { overflow: hidden; white-space: nowrap; text-overflow: ellipsis; }
  .agent-launcher-task-title { font-size: var(--fs-sm); }
  .agent-launcher-task-step { color: var(--tx-3); font-size: var(--fs-xs); }

  @keyframes agent-launcher-spin { to { rotate: 360deg; } }
  @keyframes agent-launcher-draw { to { stroke-dashoffset: 0; } }
  @keyframes agent-launcher-pulse { 50% { opacity: .35; } }
  @keyframes agent-launcher-swap { from { opacity: 0; translate: 0 4px; } }
  @keyframes agent-launcher-receive {
    0% { scale: 1; }
    35% { scale: 1.04; box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 28%, transparent); }
    100% { scale: 1; }
  }
  @keyframes agent-launcher-tasks-in { from { opacity: 0; translate: 0 -4px; } }
  @media (prefers-reduced-motion: reduce) {
    .agent-launcher-spinner, .agent-launcher-label, .agent-launcher.receiving, .agent-launcher-tasks,
    .agent-launcher-task-dot, [data-tone="attention"] .agent-launcher-dot { animation: none; }
    .agent-launcher-check { animation: none; stroke-dashoffset: 0; }
  }
</style>
