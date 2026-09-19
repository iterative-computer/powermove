/*
 * Project sessions: the host's authoritative copy of each open document.
 *
 * Every tab that has a project open is a member of its session. A tab sends
 * the patches its history records; the host applies them in arrival order,
 * stamps a sequence number, and forwards them to the other members, which
 * apply them without touching their own undo stacks. The host also writes
 * the document into the store slot the renderer autosaves to, so a tab that
 * joins later, or the agent engine, starts from the latest state.
 */
import { applyPatch, isPatchList, type Patch } from '../shared/patch';

export interface SessionMember {
  id: number;
  send(channel: string, ...args: unknown[]): void;
  isDestroyed(): boolean;
  once(event: 'destroyed', listener: () => void): unknown;
}

export interface JoinResult {
  seq: number;
  /** The host's document when it differs from what the member brought. */
  doc: unknown | null;
}

export interface PatchBroadcast { projectId: string; seq: number; patches: Patch[]; from: number }

interface Session {
  projectId: string;
  doc: any;
  seq: number;
  members: Set<SessionMember>;
  saveTimer: ReturnType<typeof setTimeout> | null;
  dirty: boolean;
}

const PROJECT_SLOT = 'project.';
const SAVE_DELAY_MS = 400;

const stringify = (value: unknown): string => JSON.stringify(value ?? null);

export class ProjectSessions {
  private readonly sessions = new Map<string, Session>();
  private readonly membership = new Map<SessionMember, Set<string>>();

  constructor(
    private readonly store: { read(key: string): unknown; set(key: string, value: unknown): void },
    private readonly options: { channel: string; version: string; log?: (line: string) => void; now?: () => number }
  ) {}

  /** A member opens a project, bringing the copy it loaded. */
  join(member: SessionMember, projectId: string, doc: unknown): JoinResult {
    let session = this.sessions.get(projectId);
    if (!session) {
      const stored = this.readSlot(projectId);
      const seed = this.fresher(stored, doc);
      session = { projectId, doc: seed, seq: 0, members: new Set(), saveTimer: null, dirty: false };
      this.sessions.set(projectId, session);
    }
    session.members.add(member);
    let owned = this.membership.get(member);
    if (!owned) {
      owned = new Set();
      this.membership.set(member, owned);
      member.once('destroyed', () => this.drop(member));
    }
    owned.add(projectId);
    // Same bytes: nothing to send. Anything else, the host's copy wins so
    // every member starts from one state.
    const same = doc != null && stringify(doc) === stringify(session.doc);
    return { seq: session.seq, doc: same ? null : session.doc };
  }

  leave(member: SessionMember, projectId: string): void {
    const session = this.sessions.get(projectId);
    if (!session) return;
    session.members.delete(member);
    this.membership.get(member)?.delete(projectId);
    if (session.members.size === 0) this.close(session);
  }

  /** Patches from one member: applied here, forwarded to the rest. */
  patch(member: SessionMember, projectId: string, patches: unknown): number {
    const session = this.sessions.get(projectId);
    if (!session || !session.members.has(member)) throw new Error('sync: not a member of that project');
    if (!isPatchList(patches)) throw new Error('sync: invalid patches');
    session.doc = applyPatch(session.doc, patches);
    session.seq += 1;
    session.dirty = true;
    const broadcast: PatchBroadcast = { projectId, seq: session.seq, patches, from: member.id };
    for (const other of session.members) {
      if (other === member || other.isDestroyed()) continue;
      other.send(this.options.channel, broadcast);
    }
    this.scheduleSave(session);
    return session.seq;
  }

  /** The current document, for the agent engine and for tests. */
  document(projectId: string): unknown | null {
    return this.sessions.get(projectId)?.doc ?? this.readSlot(projectId);
  }

  members(projectId: string): SessionMember[] {
    return [...(this.sessions.get(projectId)?.members ?? [])].filter((member) => !member.isDestroyed());
  }

  /** Replace the document wholesale (the engine after an agent run). */
  replace(projectId: string, doc: unknown, from: SessionMember | null): number {
    const session = this.sessions.get(projectId);
    if (!session) {
      this.writeSlot(projectId, doc);
      return 0;
    }
    session.doc = doc;
    session.seq += 1;
    session.dirty = true;
    const broadcast: PatchBroadcast = { projectId, seq: session.seq, patches: [{ path: [], exists: true, value: doc }], from: from?.id ?? 0 };
    for (const other of session.members) {
      if (other === from || other.isDestroyed()) continue;
      other.send(this.options.channel, broadcast);
    }
    this.scheduleSave(session);
    return session.seq;
  }

  async flush(): Promise<void> {
    for (const session of this.sessions.values()) this.save(session);
  }

  private drop(member: SessionMember): void {
    const owned = this.membership.get(member);
    this.membership.delete(member);
    for (const projectId of owned ?? []) this.leave(member, projectId);
  }

  private close(session: Session): void {
    this.save(session);
    this.sessions.delete(session.projectId);
  }

  private scheduleSave(session: Session): void {
    if (session.saveTimer) return;
    session.saveTimer = setTimeout(() => { session.saveTimer = null; this.save(session); }, SAVE_DELAY_MS);
    session.saveTimer.unref?.();
  }

  private save(session: Session): void {
    if (session.saveTimer) { clearTimeout(session.saveTimer); session.saveTimer = null; }
    if (!session.dirty) return;
    session.dirty = false;
    this.writeSlot(session.projectId, session.doc);
  }

  private readSlot(projectId: string): unknown | null {
    const raw = this.store.read(`${PROJECT_SLOT}${projectId}`) as { proj?: unknown } | null | undefined;
    return raw && typeof raw === 'object' && raw.proj ? raw.proj : null;
  }

  private writeSlot(projectId: string, doc: unknown): void {
    try {
      this.store.set(`${PROJECT_SLOT}${projectId}`, { v: this.options.version, proj: doc });
    } catch (error) {
      this.options.log?.(`[sync] could not persist ${projectId}: ${error instanceof Error ? error.message : String(error)}`);
    }
  }

  /** Higher revision wins; the member's copy is a fallback for a slot that is empty. */
  private fresher(stored: unknown | null, brought: unknown | null): unknown {
    const revision = (doc: unknown): number => (doc && typeof doc === 'object' && Number.isFinite(Number((doc as { revision?: unknown }).revision)))
      ? Number((doc as { revision?: unknown }).revision) : -1;
    if (stored == null) return brought;
    if (brought == null) return stored;
    return revision(brought) > revision(stored) ? brought : stored;
  }
}
