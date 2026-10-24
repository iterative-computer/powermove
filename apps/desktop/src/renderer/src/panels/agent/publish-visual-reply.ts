import { VISUAL_REPLY_MAX_PER_RUN, type VisualReply } from '../../../../shared/visual-reply';
import { prepareVisualReply } from './visual-reply';
import type { AgentMessage } from './agent-state.svelte';
import type { AgentThreads } from './threads';

interface RunSession {
  codexRequestId: string | null;
  projectId: string;
  threadId: string;
  conversation: AgentMessage[];
}
/** Publish into the calling run's archive, even when another thread is visible.
 * Return success only once the artifact and its message are durably saved. */
export function publishVisualReply(options: {
  runId: string; value: unknown; sessions: Iterable<RunSession>;
  busy(session: RunSession): boolean;
  archive(projectId: string): AgentThreads | undefined;
  uid(prefix: string): string;
  changed(session: RunSession): void;
}): { id: string; title: string; saved: true; threadId: string } {
  const session = [...options.sessions].find(item => item.codexRequestId === options.runId && options.busy(item));
  if (!session) throw new Error('The visualization’s agent run is no longer active.');
  const archive = options.archive(session.projectId);
  if (!archive) throw new Error('The visualization’s project is no longer open.');
  const thread = archive.threads.find(item => item.id === session.threadId);
  if (!thread) throw new Error('The visualization’s thread was removed.');
  const input = prepareVisualReply(options.value);
  const count = session.conversation.reduce((n, m) => n + (m.visualReplies || []).filter(v => v.runId === options.runId).length, 0);
  if (count >= VISUAL_REPLY_MAX_PER_RUN) throw new Error('This run already published four visual replies. Continue in a follow-up message.');
  const artifact: VisualReply = { version: 1, id: options.uid('visual-'), runId: options.runId, ...input };
  const previous = thread.conversation;
  // One attachment group per live response, rather than a full chat row for
  // every page. Preserve prose if the run already has an associated reply.
  const latest = session.conversation.at(-1);
  const group = latest?.role === 'assistant' && !latest.error
    && (!latest.visualReplies?.length || latest.visualReplies.every(reply => reply.runId === options.runId)) ? latest : undefined;
  const previousVisuals = group?.visualReplies;
  if (group) group.visualReplies = [...(previousVisuals || []), artifact];
  else session.conversation.push({ role: 'assistant', text: '', visualReplies: [artifact] });
  thread.conversation = session.conversation;
  if (!archive.save()) {
    if (group) group.visualReplies = previousVisuals;
    else session.conversation.pop();
    thread.conversation = previous;
    throw new Error('The visual reply could not be saved. Free storage and try again.');
  }
  options.changed(session);
  return { id: artifact.id, title: artifact.title, saved: true, threadId: session.threadId };
}
