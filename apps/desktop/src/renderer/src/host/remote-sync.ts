/*
 * Keeps this tab's project in step with the host's session (server/sessions.ts).
 *
 * Every change to the document, whether or not it went through history
 * (asset registration and some tool edits do not), is caught by diffing the
 * project against the last synced copy whenever the renderer signals a
 * change; the leaf patches go up to the host. Patches other tabs made come
 * down and are applied with replaceProject, which does not record history,
 * so each device keeps its own undo stack. Joining a project hands the host
 * our copy; it answers with its own when the two differ, and the host's wins.
 */
import { applyPatch, diffPatches, type Patch } from '../../../shared/patch';
import { WEB } from '../../../shared/wire';
import { bridge } from '../kernel/capture-bridge';

export interface SyncLink {
  invoke<T = unknown>(channel: string, ...args: unknown[]): Promise<T>;
  send(channel: string, ...args: unknown[]): void;
  on(channel: string, listener: (...args: unknown[]) => void): () => void;
}

const SYNC_DEBOUNCE_MS = 250;

interface RemotePatch { projectId: string; seq: number; patches: Patch[]; from: number }

type Registry = {
  RemoteSync?: { flush(): Promise<void>; refresh(): Promise<void>; ready(): Promise<void> };
  proj?: { id?: string; assets?: Record<string, { id: string }> } & Record<string, unknown>;
  assets?: { get(id: string): unknown };
  restoreProjectAssets?(project: unknown, warn?: boolean): Promise<unknown>;
  sel?: unknown;
  bus: { on(event: string, listener: (...args: any[]) => void): () => void; emit(event: string, ...args: unknown[]): void };
  replaceProject(next: unknown, options?: { selection?: unknown }): unknown;
  isHomeProject?(): boolean;
  toast?(text: string, ms?: number, options?: { error?: boolean }): void;
};

export function attachRemoteSync(link: SyncLink, PM: Registry, options: { deferWrites?: boolean } = {}): () => void {
  // A native DOM tab is a view of the authoritative worker document. During
  // extension boot, hydration can temporarily mark valid effects missing.
  // Join/pull immediately, but never publish that boot state to the worker.
  let writesHeld = options.deferWrites ?? (bridge()?.nativeHidden === true);
  let joined: string | null = null;
  let applying = false;
  let joining: Promise<void> | null = null;
  let seq = 0;
  /** JSON of the document as the host last knew it from us. */
  let synced = '';
  let flushTimer: ReturnType<typeof setTimeout> | null = null;
  let sending: Promise<void> = Promise.resolve();
  let failure: unknown = null;

  const current = (): string | null => (PM.isHomeProject?.() ? null : PM.proj?.id ?? null);
  const snapshot = (): string => { try { return JSON.stringify(PM.proj ?? null); } catch { return ''; } };

  const applyRemote = (doc: unknown, selection: unknown): void => {
    applying = true;
    try { PM.replaceProject(doc, { selection }); } finally { applying = false; }
    synced = snapshot();
    // Assets another device imported are new to this tab: load them the way a
    // project open would, from the host's media store.
    const assets = Object.values(PM.proj?.assets ?? {});
    if (PM.assets && PM.restoreProjectAssets && assets.some((asset) => !PM.assets!.get(asset.id))) {
      void PM.restoreProjectAssets(PM.proj, false).catch((error) => console.warn('[remote-sync] asset restore failed', error));
    }
  };

  /** Sends what changed since the last sync, in order, one batch at a time. */
  const flush = (): void => {
    flushTimer = null;
    const projectId = joined;
    if (writesHeld || !projectId || applying || joining || current() !== projectId) return;
    const before = synced;
    const after = snapshot();
    if (!after || after === before) return;
    let patches: Patch[];
    try { patches = diffPatches(before ? JSON.parse(before) : null, JSON.parse(after)).forward; } catch { return; }
    synced = after;
    if (!patches.length) return;
    sending = sending
      .then(() => link.invoke<number>(WEB.syncPatch, { projectId, patches }))
      .then((next) => { seq = next; }, (error) => { failure = error; console.warn('[remote-sync] patch rejected', error); });
  };
  const schedule = (): void => {
    if (writesHeld || flushTimer || !joined) return;
    flushTimer = setTimeout(flush, SYNC_DEBOUNCE_MS);
  };

  const join = (projectId: string): void => {
    if (joined === projectId) return;
    if (joined) link.send(WEB.syncLeave, joined);
    joined = projectId;
    seq = 0;
    synced = snapshot();
    const doc = PM.proj;
    joining = link.invoke<{ seq: number; doc: unknown | null }>(WEB.syncJoin, { projectId, doc })
      .then((result) => {
        if (joined !== projectId || current() !== projectId) return;
        seq = result.seq;
        if (result.doc != null) applyRemote(result.doc, PM.sel);
      })
      .catch((error) => {
        console.warn('[remote-sync] join failed', error);
        PM.toast?.('This tab is not synced with the host. Reload to retry.', 6000);
      })
      .finally(() => { joining = null; schedule(); });
  };

  const track = (): void => {
    const projectId = current();
    if (!projectId) {
      if (joined) { link.send(WEB.syncLeave, joined); joined = null; }
      return;
    }
    join(projectId);
    schedule();
  };

  const offs = [
    PM.bus.on('project', track),
    ...['layers', 'assets', 'library', 'history', 'project:recovered'].map((event) => PM.bus.on(event, schedule))
  ];
  const offRemote = link.on(WEB.syncPatch, (payload) => {
    const message = payload as RemotePatch;
    if (!message || message.projectId !== joined || current() !== message.projectId) return;
    // Anything of ours not yet sent goes first, so the remote patch lands on
    // the same base the host applied it to.
    if (flushTimer) { clearTimeout(flushTimer); flush(); }
    seq = message.seq;
    const next = applyPatch(PM.proj, message.patches);
    applyRemote(next, PM.sel);
  });

  // A new socket is a new member to the host: join again, bringing what this
  // tab has now. The host's copy still wins when the two differ.
  const offReconnect = link.on('__reconnected', () => {
    if (flushTimer) { clearTimeout(flushTimer); flushTimer = null; }
    joined = null;
    track();
  });

  // A newly booted DOM tab may have restored a stale local document after its
  // first join. Before accepting tools, discard that unsent boot state and pull
  // the current authoritative session (which may be newer than its seed).
  const refresh = async (): Promise<void> => {
    await joining; await sending;
    if (flushTimer) { clearTimeout(flushTimer); flushTimer = null; }
    if (joined) link.send(WEB.syncLeave, joined);
    joined = null; failure = null;
    track(); await joining;
  };
  const sync = { refresh, ready: async () => {
    if (!writesHeld) return;
    await refresh();
    writesHeld = false;
    synced = snapshot();
  }, flush: async () => {
    await joining;
    if (flushTimer) { clearTimeout(flushTimer); flushTimer = null; }
    flush(); await sending;
    if (failure) throw new Error(`project-sync-failed: ${String(failure)}`);
  } };
  PM.RemoteSync = sync;
  track();
  return () => {
    if (PM.RemoteSync === sync) delete PM.RemoteSync;
    for (const off of offs) off();
    offRemote();
    offReconnect();
    if (flushTimer) clearTimeout(flushTimer);
    if (joined) link.send(WEB.syncLeave, joined);
    joined = null;
  };
}
