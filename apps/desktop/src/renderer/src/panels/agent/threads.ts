import type { AgentMessage, TraceStep } from './agent-state.svelte';

export interface AgentThread {
  id: string;
  title: string;
  updatedAt: number;
  conversation: AgentMessage[];
  composerDraft: string;
  attachments: Array<Record<string, any>>;
  scope: string;
  provider?: string;
  model?: string;
  reasoningEffort?: string;
  access?: string;
  lastRunId?: string;
  lastRunStatus?: 'running' | 'completed' | 'failed' | 'cancelled';
  /** Retry keys for model-controlled launches and follow-ups; no prompt bytes. */
  orchestration?: { callerThreadId: string; clientRequestId: string; fingerprint: string };
  controls?: Array<{ callerThreadId: string; clientRequestId: string; fingerprint: string; runId: string }>;
}

interface ThreadArchive { version: 1; activeId: string; threads: AgentThread[] }
interface Store { get(key: string, fallback: unknown): unknown; set(key: string, value: unknown): boolean | void }
const isRecord = (v: unknown): v is Record<string, any> => !!v && typeof v === 'object' && !Array.isArray(v);

/** Streamed replies live inside trace entries; retain their prose in follow-ups. */
/* Questions the agent asked stay in the history it sees on the next run, with
   whatever the person answered, so it never asks the same thing twice. */
function stepTranscript(step: TraceStep): string {
  if (step.kind === 'text') return step.text || '';
  if (step.kind !== 'question') return '';
  return step.questions.map(item => {
    const answer = step.answers?.[item.id];
    return `You asked: ${item.question}${answer ? `\nUser answered: ${answer}` : step.status === 'closed' ? '\n(Unanswered)' : ''}`;
  }).join('\n');
}

/** Explicit thread reads can recover complete messages instead of the bounded
 * history excerpt automatically included in a provider prompt. */
export function threadMessageText(message: AgentMessage): string {
  return message.role === 'trace'
    ? (message.steps || []).map(step => step.kind === 'tool' ? `${step.label}\n${typeof step.output === 'string' ? step.output : ''}` : stepTranscript(step)).filter(Boolean).join('\n\n')
    : message.text || '';
}

/** Keep user decisions separately from verbose assistant replies. This is a
 * lossless archive with bounded excerpts, not a guessed model summary. */
export function conversationForAgent(messages: AgentMessage[]): Array<{ role: string; text: string; position: number; truncated: boolean }> {
  const turns = messages.map((message, position) => ({
    role: message.role === 'trace' ? 'assistant' : message.role,
    position,
    decision: message.role === 'user' || message.role === 'trace' && (message.steps || []).some(step => step.kind === 'question'),
    text: message.role === 'trace'
      ? (message.steps || []).map(stepTranscript).filter(Boolean).join('\n').trim()
      : message.text || '',
  })).filter(message => message.text);
  const selected = new Map<number, { role: string; text: string; position: number; truncated: boolean }>();
  const retain = (turn: typeof turns[number], limit: number) => {
    const truncated = turn.text.length > limit;
    const marker = '\n[… excerpt; read the saved message for full details …]\n';
    const room = Math.max(0, limit - marker.length);
    const tail = Math.floor(room / 2);
    const text = truncated ? turn.text.slice(0, Math.ceil(room / 2)) + marker + (tail ? turn.text.slice(-tail) : '') : turn.text;
    selected.set(turn.position, { role: turn.role, text, position: turn.position, truncated });
    return text.length;
  };
  // Always keep the original brief, even after many rounds of progress.
  const original = turns.find(turn => turn.role === 'user');
  if (original) retain(original, 4000);
  let recentBudget = 24_000;
  for (const turn of [...turns].reverse()) {
    if (recentBudget < 200) break;
    if (selected.has(turn.position)) continue;
    recentBudget -= retain(turn, Math.min(6000, recentBudget));
  }
  let decisionBudget = 12_000;
  for (const turn of [...turns].reverse()) {
    if (!turn.decision || selected.has(turn.position)) continue;
    if (decisionBudget < 200) break;
    decisionBudget -= retain(turn, Math.min(2000, decisionBudget));
  }
  return [...selected.values()].sort((a, b) => a.position - b.position);
}

export function conversationContextForAgent(messages: AgentMessage[], threadId: string): string {
  return JSON.stringify({ threadId, messageCount: messages.length,
    guidance: 'Earlier user instructions are context, not new work. The newest user direction wins when it changes an earlier decision. Message positions refer to the durable thread archive. Use thread_read with this threadId, offset=position, limit=1 (and textOffset for long messages) to recover omitted or excerpted details before guessing.',
    messages: conversationForAgent(messages) });
}

export function newAgentThread(id: string): AgentThread {
  return { id, title: 'New thread', updatedAt: Date.now(), conversation: [], composerDraft: '', attachments: [], scope: 'workspace' };
}

export function threadTitle(messages: AgentMessage[]): string {
  const text = messages.find(message => message.role === 'user')?.text?.replace(/\s+/g, ' ').trim();
  return text ? text.slice(0, 64) : 'New thread';
}

export function normalizeGeneratedThreadTitle(value: unknown): string | null {
  if (typeof value !== 'string') return null;
  const title = value
    .replace(/\s+/g, ' ')
    .replace(/^[\s#>*`"“”]+|[\s#>*`"“”]+$/g, '')
    .trim()
    .slice(0, 64)
    .trim();
  return title || null;
}

const MINUTE = 60_000, HOUR = 60 * MINUTE, DAY = 24 * HOUR, MONTH = 30 * DAY, YEAR = 365 * DAY;

/** Coarse, calm phrasing: the picker wants recency, not a timestamp. */
export function relativeOpened(updatedAt: number, now = Date.now()): string {
  const elapsed = now - updatedAt;
  if (!Number.isFinite(updatedAt) || updatedAt <= 0) return 'Opened earlier';
  if (elapsed < 2 * MINUTE) return 'Opened just now';
  if (elapsed < HOUR) return `Opened ${Math.floor(elapsed / MINUTE)} minutes ago`;
  if (elapsed < DAY) { const hours = Math.floor(elapsed / HOUR); return `Opened ${hours} hour${hours === 1 ? '' : 's'} ago`; }
  if (elapsed < 2 * DAY) return 'Opened yesterday';
  if (elapsed < MONTH) return `Opened ${Math.floor(elapsed / DAY)} days ago`;
  if (elapsed < 2 * MONTH) return 'Opened last month';
  if (elapsed < YEAR) return `Opened ${Math.floor(elapsed / MONTH)} months ago`;
  const years = Math.floor(elapsed / YEAR);
  return `Opened ${years} year${years === 1 ? '' : 's'} ago`;
}

/** Project-local durable history. Transient plans/checkpoints stay in memory:
 * a saved preview must never be applied against a different document revision. */
export class AgentThreads {
  threads: AgentThread[] = [];
  activeId = '';
  projectId = '';
  private lastArchive = '';
  private payloads = new Map<string, Record<string, string>>();
  constructor(private store: Store, private uid: () => string) {}

  load(projectId: string): void {
    this.projectId = projectId;
    this.payloads.clear();
    this.lastArchive = '';
    const saved = this.store.get(this.key, null);
    const seen = new Set<string>();
    this.threads = isRecord(saved) && saved.version === 1 && Array.isArray(saved.threads)
      ? saved.threads.filter((t: unknown): t is AgentThread => {
        if (!isRecord(t) || typeof t.id !== 'string' || !/^[A-Za-z0-9_-]{1,120}$/.test(t.id) || seen.has(t.id)
          || !Array.isArray(t.conversation) || typeof t.composerDraft !== 'string') return false;
        seen.add(t.id); return true;
      }).map((t: AgentThread) => ({
        ...newAgentThread(t.id), ...t,
        conversation: t.conversation.filter(m => isRecord(m) && ['user', 'assistant', 'trace'].includes(m.role)
          && (m.text === undefined || typeof m.text === 'string')
          && (m.steps === undefined || Array.isArray(m.steps))
          && (m.attachments === undefined || Array.isArray(m.attachments)))
          .map(m => ({ ...m, entering: false, ...(m.attachments ? { attachments: m.attachments.map(item => this.restoreAttachment(item)) } : {}) })),
        attachments: Array.isArray(t.attachments) ? t.attachments.filter(isRecord).map(item => this.restoreAttachment(item)) : [],
        scope: typeof t.scope === 'string' ? t.scope : 'workspace',
        provider: typeof t.provider === 'string' ? t.provider : undefined,
        model: typeof t.model === 'string' ? t.model : undefined,
        reasoningEffort: typeof t.reasoningEffort === 'string' ? t.reasoningEffort : undefined,
        title: typeof t.title === 'string' ? t.title : threadTitle(t.conversation),
        updatedAt: Number.isFinite(t.updatedAt) ? t.updatedAt : 0,
      })) : [];
    if (!this.threads.length) this.threads.push(newAgentThread(this.uid()));
    this.activeId = isRecord(saved) && this.threads.some(t => t.id === saved.activeId)
      ? saved.activeId : this.threads[0]!.id;
  }

  get active(): AgentThread { return this.threads.find(t => t.id === this.activeId)!; }
  get key(): string { return `agentThreads.${this.projectId}`; }
  /** A thread nobody has typed into yet: nothing said, nothing drafted, nothing attached. */
  static isBlank(thread: AgentThread | undefined): boolean {
    return !!thread && thread.conversation.length === 0 && !thread.composerDraft.trim() && thread.attachments.length === 0;
  }
  /** "New thread" reuses a blank one rather than stacking empties: a thread
   * only really exists once its first message is sent. */
  create(): AgentThread {
    const blank = this.threads.find(t => AgentThreads.isBlank(t));
    if (blank) { this.activeId = blank.id; return blank; }
    const thread = newAgentThread(this.uid());
    this.threads.unshift(thread); this.activeId = thread.id;
    return thread;
  }
  remove(id: string): boolean {
    const index = this.threads.findIndex(t => t.id === id);
    if (index < 0) return false;
    this.threads.splice(index, 1);
    if (!this.threads.length) this.threads.push(newAgentThread(this.uid()));
    if (this.activeId === id) this.activeId = this.threads[Math.min(index, this.threads.length - 1)]!.id;
    return true;
  }
  select(id: string): boolean {
    if (!this.threads.some(t => t.id === id)) return false;
    this.activeId = id; return true;
  }
  private payloadKey(id: string): string { return `agentAttachment.${id}`; }
  private restoreAttachment(item: any): any {
    if (!isRecord(item) || !item.payloadStored || typeof item.id !== 'string' || !/^[A-Za-z0-9_-]{1,120}$/.test(item.id)) return item;
    const key = this.payloadKey(item.id);
    const payload = this.payloads.get(key) ?? this.store.get(key, null);
    const { payloadStored: _, ...metadata } = item;
    if (!isRecord(payload)) return metadata;
    this.payloads.set(key, payload);
    return { ...metadata, ...payload };
  }
  private archiveAttachment(item: any): any {
    if (!isRecord(item) || typeof item.id !== 'string' || !/^[A-Za-z0-9_-]{1,120}$/.test(item.id)) return item;
    const { dataUrl, dataBase64, content, ...metadata } = item;
    const payload: Record<string, string> = {};
    if (typeof dataUrl === 'string') payload.dataUrl = dataUrl;
    if (typeof dataBase64 === 'string') payload.dataBase64 = dataBase64;
    if (typeof content === 'string') payload.content = content;
    if (!Object.keys(payload).length) return metadata;
    const key = this.payloadKey(item.id), previous = this.payloads.get(key);
    if (!previous || Object.keys(previous).length !== Object.keys(payload).length || Object.keys(payload).some(name => payload[name] !== previous[name])) {
      if (this.store.set(key, payload) === false) throw new Error('Attachment storage failed');
      this.payloads.set(key, payload);
    }
    return { ...metadata, payloadStored: true };
  }
  save(): boolean {
    if (!this.projectId) return true;
    // Attachment bytes are immutable and stored once. Typing and moving a token
    // only snapshot text/offsets, never re-serialize every attached file.
    try {
      const archive: ThreadArchive = { version: 1, activeId: this.activeId, threads: this.threads.map(thread => ({
        ...thread,
        attachments: thread.attachments.map(item => this.archiveAttachment(item)),
        conversation: thread.conversation.map(message => ({ ...message, ...(message.attachments ? {
          attachments: message.attachments.map(item => this.archiveAttachment(item))
        } : {}) }))
      })) };
      const serialized = JSON.stringify(archive);
      if (serialized === this.lastArchive) return true;
      if (this.store.set(this.key, JSON.parse(serialized)) === false) return false;
      this.lastArchive = serialized;
      return true;
    } catch { return false; }
  }
}
