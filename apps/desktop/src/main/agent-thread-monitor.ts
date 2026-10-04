import type { AgentThreadRun } from '../shared/agent-orchestration';
import type { CodexRunRequest, CodexRunResult, CodexTraceEvent } from '../shared/ipc';

export interface ThreadRunRecord {
  state: AgentThreadRun;
  done: Promise<AgentThreadRun>;
  resolve(state: AgentThreadRun): void;
  startingTimer?: ReturnType<typeof setTimeout>;
}
export const threadRunTerminal = (state: AgentThreadRun): boolean => ['completed', 'failed', 'cancelled', 'interrupted'].includes(state.status);

/** Watches bind to a run, not a mutable "latest turn" pointer. */
export class AgentThreadMonitor {
  private readonly records = new Map<string, ThreadRunRecord>();
  private readonly edges = new Map<string, Map<string, number>>();

  begin(request: CodexRunRequest): ThreadRunRecord {
    const record = this.reserve(request.projectId, request.threadId ?? 'legacy', request.id);
    if (threadRunTerminal(record.state)) throw new Error('This agent request has already finished. Start a new run.');
    clearTimeout(record.startingTimer);
    Object.assign(record.state, { status: 'running', providerInstanceId: request.provider ?? 'chatgpt', model: request.model, reasoningEffort: request.reasoningEffort, startedAt: Date.now() });
    this.prune();
    return record;
  }

  reserve(projectId: string, threadId: string, runId: string): ThreadRunRecord {
    const existing = this.records.get(runId);
    if (existing) {
      if (existing.state.projectId !== projectId || existing.state.threadId !== threadId) throw new Error('This run belongs to another thread.');
      return existing;
    }
    let resolve!: ThreadRunRecord['resolve'];
    const record: ThreadRunRecord = {
      state: { projectId, threadId, runId, status: 'queued', providerInstanceId: 'chatgpt', model: null, reasoningEffort: null, progress: '', summary: null, startedAt: Date.now() },
      done: new Promise(done => { resolve = done; }), resolve: state => resolve({ ...state })
    };
    // The renderer can announce a run before its IPC request reaches main.
    record.startingTimer = setTimeout(() => this.finish(record, { ok: false, error: 'The thread did not start its agent run.', cancelled: false }), 30_000);
    record.startingTimer.unref();
    this.records.set(runId, record);
    return record;
  }

  get(projectId: string, threadId: string, runId?: string): ThreadRunRecord | undefined {
    if (runId) {
      const record = this.records.get(runId);
      if (record && (record.state.projectId !== projectId || record.state.threadId !== threadId)) throw new Error('This run belongs to another thread.');
      return record;
    }
    return [...this.records.values()].filter(record => record.state.projectId === projectId && record.state.threadId === threadId).at(-1);
  }

  progress(record: ThreadRunRecord, text: string): void {
    if (threadRunTerminal(record.state)) return;
    record.state.progress = text.replace(/\s+/g, ' ').slice(0, 320);
    record.state.status = /^Waiting for (subagents|watched threads)/.test(text) ? 'waiting' : 'running';
  }

  trace(record: ThreadRunRecord, trace: CodexTraceEvent): void {
    if (trace.kind === 'tool-start') this.progress(record, trace.label);
    else if (trace.kind === 'question' && trace.blocking) { this.progress(record, 'Waiting for your answer…'); record.state.status = 'waiting'; }
  }

  finish(record: ThreadRunRecord, result: CodexRunResult): void {
    if (threadRunTerminal(record.state)) return;
    clearTimeout(record.startingTimer);
    Object.assign(record.state, { status: result.ok ? 'completed' : result.cancelled ? 'cancelled' : 'failed', summary: (result.ok ? result.text : result.error).slice(0, 200_000), endedAt: Date.now() });
    record.resolve(record.state);
  }

  watch(parentRunId: string, targetRunId: string): () => void {
    const reaches = (id: string, seen = new Set<string>()): boolean => {
      if (id === parentRunId) return true;
      if (seen.has(id)) return false;
      seen.add(id);
      return [...(this.edges.get(id)?.keys() ?? [])].some(next => reaches(next, seen));
    };
    if (reaches(targetRunId)) throw new Error('Watching this run would create a cycle.');
    const targets = this.edges.get(parentRunId) ?? new Map<string, number>();
    targets.set(targetRunId, (targets.get(targetRunId) ?? 0) + 1);
    this.edges.set(parentRunId, targets);
    let active = true;
    return () => {
      if (!active) return;
      active = false;
      const count = targets.get(targetRunId) ?? 0;
      if (count > 1) targets.set(targetRunId, count - 1); else targets.delete(targetRunId);
      if (!targets.size) this.edges.delete(parentRunId);
    };
  }

  forget(projectId: string): void {
    for (const [id, record] of this.records) if (record.state.projectId === projectId) {
      clearTimeout(record.startingTimer);
      this.records.delete(id);
    }
  }

  shutdown(): void {
    for (const record of this.records.values()) this.finish(record, { ok: false, error: 'Powermove closed before the run finished.', cancelled: true });
    this.edges.clear();
  }

  private prune(): void {
    for (const [id, record] of this.records) {
      if (this.records.size <= 512) break;
      if (threadRunTerminal(record.state) && ![...this.edges.values()].some(edges => edges.has(id))) this.records.delete(id);
    }
  }
}
