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
  | { kind: 'work'; key: string; durationMs?: number; items: ConversationItem[] };

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
    entries.push({ kind: 'work', key: `work-${start}`, items: grouped.work,
      ...(grouped.durationMs === undefined ? {} : { durationMs: grouped.durationMs }) });
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
  if (!work.length) return null;
  const durations = body.map(index => conversation[index]!)
    .filter(message => message.role === 'trace' && typeof message.durationMs === 'number')
    .map(message => message.durationMs!);
  return { work, reply, ...(durations.length ? { durationMs: Math.max(...durations) } : {}) };
}
