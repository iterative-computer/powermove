import type { AgentMessage } from './agent-state.svelte';

export interface AgentThread {
  id: string;
  title: string;
  updatedAt: number;
  conversation: AgentMessage[];
  composerDraft: string;
  attachments: Array<Record<string, any>>;
  scope: string;
}

interface ThreadArchive { version: 1; activeId: string; threads: AgentThread[] }
interface Store { get(key: string, fallback: unknown): unknown; set(key: string, value: unknown): boolean | void }
const isRecord = (v: unknown): v is Record<string, any> => !!v && typeof v === 'object' && !Array.isArray(v);
const copy = <T>(value: T): T => JSON.parse(JSON.stringify(value));

export function newAgentThread(id: string): AgentThread {
  return { id, title: 'New thread', updatedAt: Date.now(), conversation: [], composerDraft: '', attachments: [], scope: 'workspace' };
}

export function threadTitle(messages: AgentMessage[]): string {
  const text = messages.find(message => message.role === 'user')?.text?.replace(/\s+/g, ' ').trim();
  return text ? text.slice(0, 64) : 'New thread';
}

/** Project-local durable history. Transient plans/checkpoints stay in memory:
 * a saved preview must never be applied against a different document revision. */
export class AgentThreads {
  threads: AgentThread[] = [];
  activeId = '';
  projectId = '';
  constructor(private store: Store, private uid: () => string) {}

  load(projectId: string): void {
    this.projectId = projectId;
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
          .map(m => ({ ...m, entering: false })),
        attachments: Array.isArray(t.attachments) ? t.attachments.filter(isRecord) : [],
        scope: typeof t.scope === 'string' ? t.scope : 'workspace',
        title: typeof t.title === 'string' ? t.title : threadTitle(t.conversation),
        updatedAt: Number.isFinite(t.updatedAt) ? t.updatedAt : 0,
      })) : [];
    if (!this.threads.length) this.threads.push(newAgentThread(this.uid()));
    this.activeId = isRecord(saved) && this.threads.some(t => t.id === saved.activeId)
      ? saved.activeId : this.threads[0]!.id;
  }

  get active(): AgentThread { return this.threads.find(t => t.id === this.activeId)!; }
  get key(): string { return `agentThreads.${this.projectId}`; }
  create(): AgentThread {
    const thread = newAgentThread(this.uid());
    this.threads.unshift(thread); this.activeId = thread.id;
    return thread;
  }
  select(id: string): boolean {
    if (!this.threads.some(t => t.id === id)) return false;
    this.activeId = id; return true;
  }
  save(): boolean {
    if (!this.projectId) return true;
    const archive: ThreadArchive = { version: 1, activeId: this.activeId, threads: this.threads };
    // Snapshot now: asynchronous native storage must not retain mutable objects.
    try { return this.store.set(this.key, copy(archive)) !== false; } catch { return false; }
  }
}
