/*
 * Agent runs that outlive the tab that started them.
 *
 * codex/index.ts ties a run to `event.sender`: it sends progress there,
 * routes the run's tool calls there, and cancels the run when that sender is
 * destroyed. On the host the sender is a RunOwner instead of a tab. It stays
 * alive until the host stops, buffers the run's events, forwards them to
 * whichever tabs are attached, and routes each tool call to the right place:
 * document tools to the engine (or a tab while no engine is connected),
 * display tools to a live tab.
 */
import { EventEmitter } from 'node:events';

import { IPC, type AgentToolRequestEvent, type AgentToolResponseEvent, type CodexProgressEvent, type CodexRunResult } from '../shared/ipc';
import { WEB } from '../shared/wire';
import type { RemoteClient } from './clients';

/** Tools that need a real UI: pixels, panel DOM, synthetic input. */
export const DISPLAY_TOOLS = new Set([
  'get_panel_layout', 'open_panel', 'get_panel_state', 'interact_panel', 'capture_panel', 'computer_use_panel',
  'render_frames', '__panel_bounds', '__prepare_panel_input'
]);

const MAX_BUFFERED_EVENTS = 5000;

export interface RunRecord {
  id: string;
  projectId: string;
  threadId: string | null;
  provider: string;
  mode: string;
  prompt: string;
  startedAt: number;
  finishedAt: number | null;
  events: CodexProgressEvent[];
  result: CodexRunResult | null;
}

export interface RunHubDeps {
  /** The document engine's client, when connected. */
  engine(): RemoteClient | null;
  /** A live tab to show things on, most recent first. */
  tabs(): RemoteClient[];
  /** Deliver a synthetic tool response into the bridge as if the owner answered. */
  respond(owner: RunOwner, response: AgentToolResponseEvent): void;
  log?(line: string): void;
}

let nextOwnerId = -1;

/** Stands in for the WebContents that owns a run. */
export class RunOwner extends EventEmitter {
  readonly id = nextOwnerId--;
  private destroyed = false;
  private readonly attached = new Set<RemoteClient>();

  constructor(readonly record: RunRecord, private readonly hub: RunHub) { super(); }

  /* ── WebContents surface codex/index.ts and the tool bridge use ── */
  isDestroyed(): boolean { return this.destroyed; }
  isLoading(): boolean { return false; }
  getURL(): string { return 'powermove://run'; }
  get mainFrame(): { url: string } { return { url: this.getURL() }; }
  send(channel: string, payload: unknown): void {
    if (this.destroyed) return;
    if (channel === IPC.codexEvent) {
      const event = payload as CodexProgressEvent;
      this.record.events.push(event);
      if (this.record.events.length > MAX_BUFFERED_EVENTS) {
        const excess = this.record.events.length - MAX_BUFFERED_EVENTS;
        this.record.events.splice(0, excess);
        this.trimmed += excess;
      }
      for (const tab of this.attached) if (!tab.isDestroyed()) tab.send(channel, payload);
      return;
    }
    if (channel === IPC.agentToolRequest) { this.route(payload as AgentToolRequestEvent); return; }
    for (const tab of this.attached) if (!tab.isDestroyed()) tab.send(channel, payload);
  }
  capturePage(): Promise<never> { return Promise.reject(new Error('Panel capture needs an open Powermove tab.')); }
  sendInputEvent(): void { throw new Error('Synthetic input needs an open Powermove tab.'); }

  /* ── attachment: tabs that want this run's stream ─────────── */
  /**
   * `replay` catches the tab up: everything so far, or from `since` events in
   * for a tab that followed the run live until its socket dropped. The buffer
   * keeps the last MAX_BUFFERED_EVENTS, so an offset before it replays what
   * is left.
   */
  attach(tab: RemoteClient, replay: boolean | { since: number } = false): void {
    if (!this.attached.has(tab)) {
      this.attached.add(tab);
      tab.once('destroyed', () => this.attached.delete(tab));
    }
    if (!replay) return;
    const since = typeof replay === 'object' ? Math.max(0, replay.since - this.trimmed) : 0;
    for (const event of this.record.events.slice(since)) tab.send(IPC.codexEvent, event);
    if (this.record.result) tab.send(WEB.runFinished, { id: this.record.id, result: this.record.result });
  }
  /** Events dropped from the front of the buffer, so `since` offsets stay meaningful. */
  private trimmed = 0;
  detach(tab: RemoteClient): void { this.attached.delete(tab); }
  get attachedTabs(): RemoteClient[] { return [...this.attached].filter((tab) => !tab.isDestroyed()); }

  /** Ends the owner: only when the host shuts down. */
  destroy(): void {
    if (this.destroyed) return;
    this.destroyed = true;
    this.emit('destroyed');
  }

  private route(request: AgentToolRequestEvent): void {
    const target = this.hub.targetFor(request.tool, this);
    if (!target) {
      const reason = DISPLAY_TOOLS.has(request.tool)
        ? `${request.tool} needs an open Powermove tab; none is connected right now. Continue with document tools, or wait for a tab.`
        : 'No Powermove document engine or tab is connected to run this tool.';
      this.hub.deps.respond(this, { runId: request.runId, callId: request.callId, ok: false, content: [], error: reason });
      return;
    }
    this.hub.expectResponse(request.callId, this);
    target.send(IPC.agentToolRequest, { ...request, projectId: this.record.projectId });
  }
}

export class RunHub {
  private readonly owners = new Map<string, RunOwner>();
  private readonly records = new Map<string, RunRecord>();
  private readonly pendingCalls = new Map<string, RunOwner>();

  constructor(readonly deps: RunHubDeps) {}

  /** The owner for a starting run: created from the tab's request. */
  begin(request: { id: string; projectId: string; threadId?: string; provider?: string; mode: string; prompt: string }, tab: RemoteClient): RunOwner {
    const record: RunRecord = {
      id: request.id, projectId: request.projectId, threadId: request.threadId ?? null, provider: request.provider ?? 'chatgpt',
      mode: request.mode, prompt: request.prompt, startedAt: Date.now(), finishedAt: null, events: [], result: null
    };
    const owner = new RunOwner(record, this);
    owner.attach(tab);
    this.owners.set(request.id, owner);
    this.records.set(request.id, record);
    return owner;
  }

  finish(runId: string, result: CodexRunResult): void {
    const record = this.records.get(runId);
    if (!record) return;
    record.result = result;
    record.finishedAt = Date.now();
    for (const tab of this.owners.get(runId)?.attachedTabs ?? []) tab.send(WEB.runFinished, { id: runId, result });
  }

  owner(runId: string): RunOwner | null { return this.owners.get(runId) ?? null; }
  record(runId: string): RunRecord | null { return this.records.get(runId) ?? null; }

  /** Runs for a project: live ones first, then finished ones a tab may not have seen. */
  runsFor(projectId: string): RunRecord[] {
    return [...this.records.values()].filter((record) => record.projectId === projectId).sort((a, b) => b.startedAt - a.startedAt);
  }

  /** The owner that issued a tool call, so its response is credited to the owner. */
  claimResponse(callId: string): RunOwner | null {
    const owner = this.pendingCalls.get(callId) ?? null;
    if (owner) this.pendingCalls.delete(callId);
    return owner;
  }
  expectResponse(callId: string, owner: RunOwner): void { this.pendingCalls.set(callId, owner); }

  targetFor(tool: string, _owner: RunOwner): RemoteClient | null {
    if (DISPLAY_TOOLS.has(tool)) return this.deps.tabs()[0] ?? null;
    return this.deps.engine() ?? this.deps.tabs()[0] ?? null;
  }

  /** Forget finished runs older than the retention window. */
  prune(maxAgeMs = 24 * 60 * 60 * 1000, now = Date.now()): void {
    for (const [id, record] of this.records) {
      if (record.finishedAt !== null && now - record.finishedAt > maxAgeMs) {
        this.records.delete(id);
        this.owners.get(id)?.destroy();
        this.owners.delete(id);
      }
    }
  }

  shutdown(): void {
    for (const owner of this.owners.values()) owner.destroy();
    this.owners.clear();
  }
}
