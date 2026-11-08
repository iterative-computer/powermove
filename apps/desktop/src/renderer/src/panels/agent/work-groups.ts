import type { AgentMessage, TraceStep } from './agent-state.svelte';

/* A finished turn reads as prompt → "Worked for 2m 30s" → answer. Everything
   the agent did on the way (thoughts, tools, interim notes, steering) folds
   into one row the person can reopen. The turn still running stays open. */

export interface ConversationItem {
  /** Index in the conversation, for retry and answering. */
  index: number;
  message: AgentMessage;
  key: string;
}

export type ConversationEntry =
  | ({ kind: 'message' } & ConversationItem)
  | { kind: 'work'; key: string; durationMs?: number; items: ConversationItem[]; subagents?: { total: number; failed: number } };

/** Subagents that worked inside folded messages, by their latest state. */
export function subagentCounts(items: ConversationItem[]): { total: number; failed: number } | undefined {
  const latest = new Map<string, string>();
  for (const { message } of items) for (const step of message.steps ?? []) {
    if (step.kind === 'tool' && step.task) latest.set(step.task.taskId, step.task.status);
  }
  if (!latest.size) return undefined;
  const failed = [...latest.values()].filter(status => status === 'failed' || status === 'interrupted').length;
  return { total: latest.size, failed };
}

/** "Worked for 2m 30s · 3 subagents, 1 failed". */
export function workLabel(entry: { durationMs?: number; subagents?: { total: number; failed: number } }): string {
  const base = entry.durationMs === undefined ? 'Work' : `Worked for ${workDuration(entry.durationMs)}`;
  if (!entry.subagents) return base;
  const { total, failed } = entry.subagents;
  return `${base} · ${total} ${total === 1 ? 'subagent' : 'subagents'}${failed ? `, ${failed} failed` : ''}`;
}

export function workDuration(ms: number): string {
  const seconds = Math.max(0, Math.floor(ms / 1000));
  return seconds < 60 ? `${seconds}s`
    : seconds < 3600 ? `${Math.floor(seconds / 60)}m ${seconds % 60}s`
    : `${Math.floor(seconds / 3600)}h ${Math.floor(seconds % 3600 / 60)}m ${seconds % 60}s`;
}

/** Splits a trace's trailing prose (the reply when the agent spoke) from the work before it. */
function splitTrailingText(steps: TraceStep[]): { work: TraceStep[]; reply: TraceStep[] } {
  let cut = steps.length;
  while (cut > 0 && steps[cut - 1]!.kind === 'text') cut--;
  return { work: steps.slice(0, cut), reply: steps.slice(cut) };
}

const item = (index: number, message: AgentMessage, part = ''): ConversationItem =>
  ({ index, message, key: `${message.role}-${index}${part}` });

/**
 * Groups the conversation for display. `liveFrom` is any conversation index
 * inside the running turn (its prompt or a steer), or null when nothing runs.
 */
export function groupConversation(conversation: AgentMessage[], liveFrom: number | null): ConversationEntry[] {
  const entries: ConversationEntry[] = [];
  const plain = (index: number): void => { entries.push({ kind: 'message', ...item(index, conversation[index]!) }); };
  let index = 0;
  // Anything before the first prompt (restored notices) stays as written.
  while (index < conversation.length && !(conversation[index]!.role === 'user' && !conversation[index]!.steering)) plain(index++);

  while (index < conversation.length) {
    const start = index;
    let end = start + 1;
    while (end < conversation.length && !(conversation[end]!.role === 'user' && !conversation[end]!.steering)) end++;
    index = end;
    plain(start);
    const body = Array.from({ length: end - start - 1 }, (_, offset) => start + 1 + offset);
    const live = liveFrom !== null && liveFrom >= start && liveFrom < end;
    const grouped = live ? null : foldTurn(conversation, body);
    if (!grouped) { body.forEach(plain); continue; }
    const subagents = subagentCounts(grouped.work);
    entries.push({ kind: 'work', key: `work-${start}`, items: grouped.work,
      ...(grouped.durationMs === undefined ? {} : { durationMs: grouped.durationMs }), ...(subagents ? { subagents } : {}) });
    for (const reply of grouped.reply) entries.push({ kind: 'message', ...reply });
  }
  return entries;
}

function foldTurn(conversation: AgentMessage[], body: number[]): { work: ConversationItem[]; reply: ConversationItem[]; durationMs?: number } | null {
  const lastIndex = body.at(-1);
  if (lastIndex === undefined) return null;
  const last = conversation[lastIndex]!;
  const work: ConversationItem[] = [];
  const reply: ConversationItem[] = [];
  let workEnd = body.length;

  if (last.role === 'assistant') {
    reply.push(item(lastIndex, last));
    workEnd = body.length - 1;
    // When the agent spoke in the trace, the assistant turn only adds notes:
    // the prose just above it is part of the answer.
    const beforeIndex = body[workEnd - 1];
    const before = beforeIndex === undefined ? undefined : conversation[beforeIndex]!;
    if (beforeIndex !== undefined && before?.role === 'trace' && !before.steering) {
      const { work: steps, reply: text } = splitTrailingText(before.steps ?? []);
      if (text.length) {
        workEnd -= 1;
        if (steps.length) work.push(item(beforeIndex, { ...before, steps }, ':work'));
        reply.unshift(item(beforeIndex, { ...before, steps: text }, ':reply'));
      }
    }
  } else if (last.role === 'trace') {
    const { work: steps, reply: text } = splitTrailingText(last.steps ?? []);
    // A run that ended without saying anything keeps its work in view.
    if (!text.length) return null;
    workEnd = body.length - 1;
    if (steps.length) work.push(item(lastIndex, { ...last, steps }, ':work'));
    reply.push(item(lastIndex, { ...last, steps: text }, ':reply'));
  } else {
    return null;
  }

  const earlier = body.slice(0, workEnd).map(index => item(index, conversation[index]!));
  work.unshift(...earlier);
  // Published artifacts are deliverables, even when a later assistant message
  // supplies the final prose. Keep their original top-level keys so finishing
  // the run neither hides nor remounts their interactive frames.
  const artifacts = work.filter(entry => entry.message.visualReplies?.length);
  for (let index = work.length - 1; index >= 0; index--) {
    if (work[index]!.message.visualReplies?.length) work.splice(index, 1);
  }
  reply.unshift(...artifacts);
  if (!work.length) return null;
  const durations = body.map(index => conversation[index]!)
    .filter(message => message.role === 'trace' && typeof message.durationMs === 'number')
    .map(message => message.durationMs!);
  return { work, reply, ...(durations.length ? { durationMs: Math.max(...durations) } : {}) };
}
