/*
 * Keeps this tab's project in step with the host's session (server/sessions.ts).
 *
 * Every history entry already publishes the leaf patches it made
 * (`history:project-patch`, forward on commit and redo, backward on undo).
 * Those go up to the host; patches other tabs made come down and are applied
 * with replaceProject, which does not record history, so each device keeps
 * its own undo stack. Joining a project hands the host our copy; it answers
 * with its own when the two differ, and the host's wins.
 */
import { applyPatch, type Patch } from '../../../shared/patch';
import { WEB } from '../../../shared/wire';

export interface SyncLink {
  invoke<T = unknown>(channel: string, ...args: unknown[]): Promise<T>;
  send(channel: string, ...args: unknown[]): void;
  on(channel: string, listener: (...args: unknown[]) => void): () => void;
}

interface RemotePatch { projectId: string; seq: number; patches: Patch[]; from: number }

type Registry = {
  proj?: { id?: string } & Record<string, unknown>;
  sel?: unknown;
  bus: { on(event: string, listener: (...args: any[]) => void): () => void; emit(event: string, ...args: unknown[]): void };
  replaceProject(next: unknown, options?: { selection?: unknown }): unknown;
  isHomeProject?(): boolean;
  toast?(text: string, ms?: number, options?: { error?: boolean }): void;
};

export function attachRemoteSync(link: SyncLink, PM: Registry): () => void {
  let joined: string | null = null;
  let applying = false;
  let joining: Promise<void> | null = null;
  let seq = 0;

  const current = (): string | null => (PM.isHomeProject?.() ? null : PM.proj?.id ?? null);

  const applyRemote = (doc: unknown, selection: unknown): void => {
    applying = true;
    try { PM.replaceProject(doc, { selection }); } finally { applying = false; }
  };

  const join = (projectId: string): void => {
    if (joined === projectId) return;
    if (joined) link.send(WEB.syncLeave, joined);
    joined = projectId;
    seq = 0;
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
      .finally(() => { joining = null; });
  };

  const track = (): void => {
    const projectId = current();
    if (!projectId) {
      if (joined) { link.send(WEB.syncLeave, joined); joined = null; }
      return;
    }
    join(projectId);
  };

  const offProject = PM.bus.on('project', track);
  const offPatch = PM.bus.on('history:project-patch', (entry: { projectId?: string; patches?: Patch[] }) => {
    if (applying || !entry?.projectId || entry.projectId !== joined || !entry.patches?.length) return;
    const send = () => link.invoke<number>(WEB.syncPatch, { projectId: entry.projectId, patches: entry.patches })
      .then((next) => { seq = next; })
      .catch((error) => console.warn('[remote-sync] patch rejected', error));
    // A join in flight means our copy may be about to be replaced; queue behind it.
    if (joining) void joining.then(send); else void send();
  });
  const offRemote = link.on(WEB.syncPatch, (payload) => {
    const message = payload as RemotePatch;
    if (!message || message.projectId !== joined || current() !== message.projectId) return;
    seq = message.seq;
    const next = applyPatch(PM.proj, message.patches);
    applyRemote(next, PM.sel);
  });

  track();
  return () => { offProject(); offPatch(); offRemote(); if (joined) link.send(WEB.syncLeave, joined); joined = null; };
}
