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

export function taskTrace(task: AgentTask): Extract<TraceStep, { kind: 'tool' }> {
  const provider = task.providerInstanceId === 'claude' ? 'Claude' : task.providerInstanceId === 'chatgpt' ? 'ChatGPT' : 'Connected model';
  const model = AGENT_MODELS[task.providerInstanceId].find(model => model.id === task.model)?.label ?? task.model;
  const live = ['queued', 'running', 'waiting'].includes(task.status);
  return {
    kind: 'tool', id: task.taskId, toolName: 'delegate_task', label: task.title,
    detail: [provider, model].filter(Boolean).join(' · '), output: taskResultText(task),
    status: live ? 'running' : task.status === 'failed' || task.status === 'interrupted' ? 'error' : 'done',
    startedAt: task.startedAt, endedAt: task.endedAt, task: { ...task }
  };
}
