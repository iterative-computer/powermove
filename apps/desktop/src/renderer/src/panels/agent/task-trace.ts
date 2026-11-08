import type { AgentTask } from '../../../../shared/agent-orchestration';
import type { TraceStep } from './agent-state.svelte';
import { AGENT_MODELS } from '../../../../shared/agent-models';

export function taskResultText(task: AgentTask): string {
  if (!task.summary) return task.progress;
  try {
    const result = JSON.parse(task.summary);
    if (typeof result?.summary === 'string') return [result.summary, ...(Array.isArray(result.notes) ? result.notes.filter((note: unknown) => typeof note === 'string') : [])].join('\n\n');
  } catch { /* Planning providers return plain text. */ }
  return task.summary;
}

export function taskProvider(task: Pick<AgentTask, 'providerInstanceId'>): string {
  return task.providerInstanceId === 'claude' ? 'Claude' : task.providerInstanceId === 'chatgpt' ? 'ChatGPT' : 'Connected model';
}

/** A model a person can read: the picker's label, else a tidied id
 * ("claude-haiku-5-5" → "Haiku 5.5"), never a raw slug. */
export function taskModel(task: Pick<AgentTask, 'providerInstanceId' | 'model'>): string {
  if (!task.model) return '';
  const known = AGENT_MODELS[task.providerInstanceId]?.find(model => model.id === task.model)?.label;
  if (known) return known;
  if (!/^[a-z0-9.-]+$/i.test(task.model)) return task.model;
  return task.model.replace(/^claude-/i, '').replace(/-(\d+)-(\d+)(?=$|-)/g, ' $1.$2').split(/[-\s]+/)
    .map(word => /^gpt$/i.test(word) ? 'GPT' : word.replace(/^./, c => c.toUpperCase())).join(' ').replace(/^GPT (\d)/, 'GPT-$1');
}

export type TaskPhase = 'working' | 'waiting' | 'done' | 'failed' | 'stopped';
export function taskPhase(task: Pick<AgentTask, 'status'>): TaskPhase {
  return task.status === 'completed' ? 'done'
    : task.status === 'failed' || task.status === 'interrupted' ? 'failed'
    : task.status === 'cancelled' ? 'stopped'
    : task.status === 'waiting' ? 'waiting' : 'working';
}

/** Parents first, each followed by its subagents, with their depth. */
export function taskTree(tasks: AgentTask[]): Array<{ task: AgentTask; depth: number }> {
  const latest = new Map<string, AgentTask>();
  for (const task of tasks) latest.set(task.taskId, task);
  const all = [...latest.values()];
  const ids = new Set(all.map(task => task.taskId));
  const out: Array<{ task: AgentTask; depth: number }> = [];
  const visit = (task: AgentTask, depth: number) => {
    out.push({ task, depth });
    for (const child of all.filter(item => item.parentTaskId === task.taskId)) if (depth < 4) visit(child, depth + 1);
  };
  for (const task of all) if (!task.parentTaskId || !ids.has(task.parentTaskId)) visit(task, 0);
  return out;
}

export function taskTrace(task: AgentTask): Extract<TraceStep, { kind: 'tool' }> {
  const provider = taskProvider(task);
  const model = taskModel(task);
  const live = ['queued', 'running', 'waiting'].includes(task.status);
  return {
    kind: 'tool', id: task.taskId, toolName: 'delegate_task', label: task.title,
    detail: [provider, model].filter(Boolean).join(' · '), output: taskResultText(task),
    status: live ? 'running' : task.status === 'failed' || task.status === 'interrupted' ? 'error' : 'done',
    startedAt: task.startedAt, endedAt: task.endedAt, task: { ...task }
  };
}
