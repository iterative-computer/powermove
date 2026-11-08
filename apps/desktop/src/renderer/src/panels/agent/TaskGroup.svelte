<script lang="ts">
  import { onDestroy } from 'svelte';
  import type { AgentTask } from '../../../../shared/agent-orchestration';
  import type { TasksRow } from './activity-rows';
  import { durationLabel, thoughtLead } from './activity-rows';
  import { taskModel, taskPhase, taskProvider, taskResultText, taskTree, type TaskPhase } from './task-trace';
  import Markdown from './Markdown.svelte';
  import { bridge } from '../../kernel/bridge';
  import { agentState } from './agent-state.svelte';

  /* Subagents the agent handed work to, as one small team card. Each row says
     who (provider and model), what (the brief's title), and how it is going
     right now (live progress, the answer's opening, or why it failed). Each
     subagent works in a sub-thread of this one: a row opens it, to watch the
     work or message the subagent. Subagents a subagent started sit beneath it. */

  let { row, animated = false, PM }: { row: TasksRow; animated?: boolean; PM?: Record<string, any> } = $props();
  const subthread = (task: AgentTask): string | undefined => {
    const id = `sub-${task.taskId}`;
    return PM?.AgentUI?.switchThread && (agentState.threads || []).some(thread => thread.id === id) ? id : undefined;
  };

  const items = $derived(taskTree(row.tasks));
  const live = (task: AgentTask) => ['working', 'waiting'].includes(taskPhase(task));
  const anyLive = $derived(items.some(item => live(item.task)));

  // One shared clock for every running timer in the card.
  let now = $state(Date.now());
  let clock: ReturnType<typeof setInterval> | undefined;
  $effect(() => {
    if (!anyLive) { if (clock) { clearInterval(clock); clock = undefined; } return; }
    now = Date.now();
    clock ??= setInterval(() => { now = Date.now(); }, 1000);
  });
  onDestroy(() => { if (clock) clearInterval(clock); });

  const counts = $derived(items.reduce((sum, { task }) => {
    const phase = taskPhase(task);
    sum[phase === 'waiting' ? 'working' : phase] += 1;
    return sum;
  }, { working: 0, done: 0, failed: 0, stopped: 0 } as Record<'working' | 'done' | 'failed' | 'stopped', number>));
  const title = $derived(`${items.length} ${items.length === 1 ? 'subagent' : 'subagents'}`);
  const summary = $derived([
    counts.working ? `${counts.working} working` : '',
    counts.done ? `${counts.done} done` : '',
    counts.failed ? `${counts.failed} failed` : '',
    counts.stopped ? `${counts.stopped} stopped` : ''
  ].filter(Boolean).join(' · '));

  const elapsed = (task: AgentTask): string => {
    const end = task.endedAt ?? (live(task) ? now : undefined);
    return end === undefined ? '' : durationLabel(Math.max(0, end - task.startedAt));
  };
  const who = (task: AgentTask): string => [taskProvider(task), taskModel(task)].filter(Boolean).join(' ');
  const PHASE_WORD: Record<TaskPhase, string> = { working: 'Working', waiting: 'Waiting on its subagents', done: 'Done', failed: 'Failed', stopped: 'Stopped' };
  /* The second line: what it is doing now, or the opening of what it found. */
  function line(task: AgentTask): string {
    const phase = taskPhase(task);
    if (phase === 'waiting') return PHASE_WORD.waiting;
    if (phase === 'working') return task.progress?.trim() || (task.status === 'queued' ? 'Starting…' : 'Getting started…');
    const result = taskResultText(task).trim();
    if (phase === 'stopped') return result && !/^the task was stopped\.?$/i.test(result) ? thoughtLead(result) : 'Stopped before it finished';
    return result ? thoughtLead(result) : PHASE_WORD[phase];
  }

  let manual = $state<Record<string, boolean>>({});
  const full = (task: AgentTask): string => taskResultText(task).trim();
  // Without a sub-thread (older runs), a row unfolds the full answer in place.
  const canOpen = (task: AgentTask): boolean => !subthread(task) && !live(task) && full(task).length > thoughtLead(full(task)).length;
  const isOpen = (task: AgentTask): boolean => canOpen(task) && (manual[task.taskId] ?? false);

  let stopping = $state<Record<string, boolean>>({});
  let stopErrors = $state<Record<string, string>>({});
  async function stop(task: AgentTask): Promise<void> {
    if (stopping[task.taskId]) return;
    stopping[task.taskId] = true;
    stopErrors[task.taskId] = '';
    try {
      const cancel = bridge()?.codex.cancelTask;
      if (!cancel) throw new Error('Subagent controls are unavailable in this connection.');
      await cancel(task.requestId, task.taskId);
    } catch (error) { stopErrors[task.taskId] = error instanceof Error ? error.message : 'Could not stop this subagent.'; }
    finally { stopping[task.taskId] = false; }
  }
</script>

<section class="agent-subagents" class:is-live={anyLive} aria-label="Subagents">
  <header>
    <svg class="agent-subagents-icon" viewBox="0 0 24 24" aria-hidden="true"><path d="M9 11a4 4 0 1 0 0-8 4 4 0 0 0 0 8zM2 21v-1a6 6 0 0 1 6-6h2a6 6 0 0 1 6 6v1M16 3.1a4 4 0 0 1 0 7.8M22 21v-1a6 6 0 0 0-4-5.6" /></svg>
    <span class="agent-subagents-title">{title}</span>
    <span class="agent-subagents-summary">{summary}</span>
  </header>
  <div class="agent-subagents-meter" aria-hidden="true">
    {#each items as { task } (task.taskId)}<i data-phase={taskPhase(task)}></i>{/each}
  </div>
  <ul>
    {#each items as { task, depth } (task.taskId)}
      {@const phase = taskPhase(task)}
      {@const open = isOpen(task)}
      {@const thread = subthread(task)}
      <li data-phase={phase} class:is-nested={depth > 0} style:--depth={depth}>
        <div class="agent-subagent-line">
          <button type="button" class="agent-subagent-row" aria-expanded={canOpen(task) ? open : undefined} disabled={!thread && !canOpen(task)}
            title={thread ? 'Open this subagent’s sub-thread' : undefined}
            aria-label={thread ? `Open ${task.title}` : undefined}
            onclick={() => { if (thread) PM?.AgentUI?.switchThread(thread); else manual = { ...manual, [task.taskId]: !open }; }}>
            <span class="agent-subagent-status" aria-label={PHASE_WORD[phase]} role="img">
              {#if phase === 'working'}<i class="agent-subagent-spinner"></i>
              {:else if phase === 'waiting'}<svg viewBox="0 0 24 24"><circle cx="12" cy="12" r="8" stroke-dasharray="3 3.3" /></svg>
              {:else if phase === 'done'}<svg viewBox="0 0 24 24"><path d="M5 12.5l4.5 4.5L19 7.5" /></svg>
              {:else if phase === 'failed'}<svg viewBox="0 0 24 24"><path d="M12 7v6M12 16.5v.5" /><circle cx="12" cy="12" r="9" /></svg>
              {:else}<svg viewBox="0 0 24 24"><rect x="7" y="7" width="10" height="10" rx="1.5" /></svg>{/if}
            </span>
            <span class="agent-subagent-text">
              <span class="agent-subagent-name">{task.title}</span>
              <span class="agent-subagent-sub">
                <span class="agent-subagent-who">{who(task)}</span>
                <span class="agent-subagent-what" class:shimmer-text={animated && phase === 'working'}>{line(task)}</span>
              </span>
            </span>
            <span class="agent-subagent-time">{elapsed(task)}</span>
            {#if thread}<svg class="agent-subagent-open" viewBox="0 0 24 24" aria-hidden="true"><path d="M9 6l6 6-6 6" /></svg>
            {:else if canOpen(task)}<svg class="agent-subagent-chevron" viewBox="0 0 24 24" aria-hidden="true"><path d="M6 9l6 6 6-6" /></svg>{/if}
          </button>
          {#if live(task)}
            <button type="button" class="agent-subagent-stop" aria-label={`Stop ${task.title}`} title="Stop this subagent"
              disabled={stopping[task.taskId]} onclick={() => void stop(task)}>
              <svg viewBox="0 0 24 24" aria-hidden="true"><rect x="7" y="7" width="10" height="10" rx="1.5" /></svg>
            </button>
          {/if}
        </div>
        {#if stopErrors[task.taskId]}<p class="agent-subagent-error" role="alert">{stopErrors[task.taskId]}</p>{/if}
        {#if open}
          <div class="agent-subagent-result"><Markdown text={full(task)} /></div>
        {/if}
      </li>
    {/each}
  </ul>
</section>

<style>
  .agent-subagents {
    min-width: 0;
    margin: 2px 0;
    border-radius: var(--r-md);
    background: var(--ink-0, transparent);
    box-shadow: inset 0 0 0 var(--hairline) var(--line-2);
    font-size: var(--fs-sm);
    color: var(--tx-2);
    overflow: hidden;
  }
  header { display: flex; align-items: center; gap: 7px; min-width: 0; padding: 8px 10px 6px; }
  .agent-subagents-icon { width: 14px; height: 14px; flex: none; fill: none; stroke: var(--tx-3); stroke-width: 1.8; stroke-linecap: round; stroke-linejoin: round; }
  .agent-subagents-title { font-weight: var(--fw-medium); color: var(--tx); }
  .agent-subagents-summary { margin-left: auto; color: var(--tx-3); font-size: var(--fs-xs); font-variant-numeric: tabular-nums; white-space: nowrap; }

  /* One segment per subagent: a glance at how far the team has got. */
  .agent-subagents-meter { display: flex; gap: 3px; height: 3px; margin: 0 10px 4px; }
  .agent-subagents-meter > i { flex: 1; border-radius: 2px; background: var(--line-2); transition: background var(--dur-2); }
  .agent-subagents-meter > i[data-phase="working"], .agent-subagents-meter > i[data-phase="waiting"] { background: var(--tx-3); }
  .agent-subagents-meter > i[data-phase="done"] { background: var(--success); }
  .agent-subagents-meter > i[data-phase="failed"] { background: var(--danger); }
  .agent-subagents.is-live .agent-subagents-meter > i[data-phase="working"] { animation: agent-subagent-pulse 1.6s ease-in-out infinite; }
  @keyframes agent-subagent-pulse { 50% { opacity: .45; } }

  ul { list-style: none; margin: 0; padding: 0 4px 4px; }
  li { position: relative; min-width: 0; }
  li + li { border-top: 1px solid var(--line); }
  li.is-nested { margin-left: calc(var(--depth) * 18px); border-top: 0; }
  /* The connector: a subagent's subagents hang off its status column. */
  li.is-nested::before { content: ""; position: absolute; left: -9px; top: 0; height: 19px; width: 8px; border-left: 1px solid var(--line-2); border-bottom: 1px solid var(--line-2); border-bottom-left-radius: 5px; }

  .agent-subagent-line { display: flex; align-items: flex-start; min-width: 0; }
  .agent-subagent-row {
    display: grid; grid-template-columns: 16px minmax(0, 1fr) auto auto; align-items: start; column-gap: 8px;
    flex: 1; min-width: 0; margin: 0; padding: 7px 6px; border: 0; border-radius: var(--r-sm);
    background: transparent; color: inherit; font: inherit; text-align: left; cursor: default;
    transition: background var(--dur-2);
  }
  .agent-subagent-row:not(:disabled) { cursor: pointer; }
  .agent-subagent-row:not(:disabled):hover, .agent-subagent-row[aria-expanded="true"] { background: var(--ink-1); }
  .agent-subagent-row:focus-visible { outline: 1px solid var(--accent); outline-offset: -1px; }
  .agent-subagent-row:disabled { opacity: 1; }

  .agent-subagent-status { display: grid; place-items: center; width: 16px; height: 18px; }
  .agent-subagent-status svg { width: 15px; height: 15px; fill: none; stroke: currentColor; stroke-width: 2; stroke-linecap: round; stroke-linejoin: round; }
  [data-phase="done"] .agent-subagent-status { color: var(--success); }
  [data-phase="failed"] .agent-subagent-status { color: var(--danger); }
  [data-phase="waiting"] .agent-subagent-status, [data-phase="stopped"] .agent-subagent-status { color: var(--tx-3); }
  [data-phase="waiting"] .agent-subagent-status svg { animation: agent-subagent-turn 6s linear infinite; }
  @keyframes agent-subagent-turn { to { transform: rotate(360deg); } }
  .agent-subagent-spinner { width: 12px; height: 12px; border-radius: 50%; border: 1.5px solid var(--line-2); border-top-color: var(--tx); animation: agent-subagent-turn .8s linear infinite; }

  .agent-subagent-text { display: flex; flex-direction: column; gap: 1px; min-width: 0; }
  .agent-subagent-name { color: var(--tx); font-weight: var(--fw-medium); line-height: 18px; overflow-wrap: anywhere; }
  [data-phase="stopped"] .agent-subagent-name { color: var(--tx-2); }
  .agent-subagent-sub { display: flex; gap: 6px; min-width: 0; font-size: var(--fs-xs); line-height: 16px; color: var(--tx-3); }
  .agent-subagent-who { flex: none; color: var(--tx-2); }
  .agent-subagent-who:empty { display: none; }
  .agent-subagent-who:not(:empty)::after { content: "·"; margin-left: 6px; color: var(--tx-4, var(--tx-3)); }
  .agent-subagent-what { min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  /* Why it failed is the one thing worth reading in full. */
  [data-phase="failed"] .agent-subagent-what { color: var(--danger); white-space: normal; overflow-wrap: anywhere; }
  [data-phase="failed"] .agent-subagent-sub { flex-direction: column; gap: 0; }
  [data-phase="failed"] .agent-subagent-who::after { content: none; }
  .agent-subagent-time { color: var(--tx-3); font-size: var(--fs-xs); line-height: 18px; font-variant-numeric: tabular-nums; white-space: nowrap; }
  .agent-subagent-chevron { width: 12px; height: 12px; margin-top: 3px; fill: none; stroke: var(--tx-3); stroke-width: 2.2; stroke-linecap: round; stroke-linejoin: round; transform: rotate(-90deg); transition: transform 200ms var(--ease); }
  [aria-expanded="true"] > .agent-subagent-chevron { transform: rotate(0deg); }
  .agent-subagent-open { width: 12px; height: 12px; margin-top: 3px; flex: none; fill: none; stroke: var(--tx-3); stroke-width: 2.2; stroke-linecap: round; stroke-linejoin: round; opacity: .7; }
  .agent-subagent-row:hover > .agent-subagent-open { opacity: 1; stroke: var(--tx-2); }

  .agent-subagent-stop {
    display: grid; place-items: center; flex: none; width: 24px; height: 24px; margin: 4px 2px 0 0; padding: 0;
    border: 0; border-radius: var(--r-sm); background: transparent; color: var(--tx-3); cursor: pointer;
    transition: background var(--dur-2), color var(--dur-2);
  }
  .agent-subagent-stop svg { width: 13px; height: 13px; fill: currentColor; }
  .agent-subagent-stop:hover { background: color-mix(in srgb, var(--danger) 14%, transparent); color: var(--danger); }
  .agent-subagent-stop:focus-visible { outline: 1px solid var(--accent); outline-offset: -1px; }
  .agent-subagent-stop:disabled { opacity: .5; cursor: default; }
  .agent-subagent-error { margin: 0 8px 6px 30px; color: var(--danger); font-size: var(--fs-xs); }
  .agent-subagent-result {
    margin: 0 8px 8px 29px; padding: 2px 0 2px 12px; border-left: 1px solid var(--line-2);
    color: var(--tx-2); font-size: var(--fs-sm); line-height: 1.55; overflow-wrap: anywhere;
    user-select: text; -webkit-user-select: text;
  }

  @media (prefers-reduced-motion: reduce) {
    .agent-subagent-spinner, [data-phase="waiting"] .agent-subagent-status svg, .agent-subagents-meter > i { animation: none !important; }
  }
</style>
