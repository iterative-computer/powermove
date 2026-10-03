import { thoughtLead } from './activity-rows';
import type { AgentMessage, AgentPhase, TraceStep } from './agent-state.svelte';

/** What the titlebar launcher says about the agent at a glance. */
export type LauncherTone = 'idle' | 'working' | 'attention' | 'review' | 'done' | 'error';

export interface LauncherStatus {
  tone: LauncherTone;
  label: string;
  /** When the visible run began, for the elapsed time. */
  startedAt: number | null;
  /** Runs in progress across all of this project's threads. */
  tasks: number;
}

export interface LauncherInput {
  phase: AgentPhase;
  trace: TraceStep[];
  activity: string;
  conversation: AgentMessage[];
  backgroundRuns?: number;
  runStartedAt?: number | null;
  workingStartedAt?: number | null;
  panelRun?: unknown;
  run?: { autonomous?: boolean } | null;
  /** The running thread's name, for runs that report no steps. */
  threadTitle?: string;
}

/** The step the run is on right now, newest first. */
export function currentStepLabel(trace: TraceStep[] = []): string {
  for (let index = trace.length - 1; index >= 0; index--) {
    const step = trace[index]!;
    if (step.kind === 'tool' && step.status === 'running') return step.label || 'Using a tool';
    if (step.kind === 'thought' && step.live) return thoughtLead(step.label) || 'Thinking';
  }
  return '';
}

/** One plain line from a reply: no markdown, no trailing sandbox report. */
export function replyLead(text = ''): string {
  const line = text.split(/\n+/).map(part => part.replace(/[*_`#>]+/g, '').trim()).find(Boolean) ?? '';
  return line.length > 72 ? `${line.slice(0, 71).trimEnd()}…` : line;
}

export function launcherStatus(state: LauncherInput, unseen: boolean): LauncherStatus {
  const background = Math.max(0, state.backgroundRuns ?? 0);
  const running = state.phase === 'running';
  const tasks = background + (running ? 1 : 0);
  const startedAt = state.runStartedAt ?? state.workingStartedAt ?? null;
  if (running) {
    const question = state.trace.some(step => step.kind === 'question' && step.status === 'open' && step.blocking);
    if (question) return { tone: 'attention', label: 'Needs your answer', startedAt, tasks };
    return { tone: 'working', label: currentStepLabel(state.trace) || state.activity.replace(/…$/, '') || state.threadTitle || 'Working', startedAt, tasks };
  }
  if (state.phase === 'preview') return { tone: 'review', label: 'Review the proposed change', startedAt: null, tasks };
  if (state.phase === 'result' && (state.panelRun || (state.run && !state.run.autonomous))) {
    return { tone: 'review', label: 'Review the changes', startedAt: null, tasks };
  }
  const last = state.conversation.at(-1);
  if (unseen && last?.role === 'assistant') {
    if (last.error && last.notice !== 'plain') return { tone: 'error', label: replyLead(last.text) || 'The run stopped', startedAt: null, tasks };
    return { tone: 'done', label: replyLead(last.text) || 'Done', startedAt: null, tasks };
  }
  if (background) return { tone: 'working', label: `${background} ${background === 1 ? 'task' : 'tasks'} running`, startedAt: null, tasks };
  return { tone: 'idle', label: 'Ask the agent', startedAt: null, tasks };
}

/** "12s", "3:04", "1:02:09": compact enough for the titlebar. */
export function elapsedLabel(ms: number): string {
  const seconds = Math.max(0, Math.floor(ms / 1000));
  if (seconds < 60) return `${seconds}s`;
  const s = String(seconds % 60).padStart(2, '0');
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}:${s}`;
  return `${Math.floor(minutes / 60)}:${String(minutes % 60).padStart(2, '0')}:${s}`;
}
