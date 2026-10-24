import type { PowermoveAPI } from 'powermove';

/*
 * The API probe for the Store sandbox (owner amendment 24), at apiVersion 1,
 * 2 and 3 (the test writes the manifests). A sandboxed extension runs in an
 * opaque iframe: the members EXTENSIONS.md lists as returning Promises do so,
 * the reactive reads stay synchronous, `project.get()` resolves to a frozen
 * snapshot, and trusted-only namespaces are not there. The command returns
 * what it saw over the sandbox's RPC; the test compares the engine's answers
 * (snapshots, edits) with what the editor itself reads.
 */
type Result = { ok: boolean; detail?: unknown };

export default function activate(api: PowermoveAPI): void {
  const seen: Record<string, number> = {};
  // The editor events a sandbox gets (sandbox-host.ts HOST_EVENTS; `theme` is
  // `theme:changed`); frame and overlay events never reach a sandbox.
  for (const name of ['project:changed', 'selection', 'time', 'transport', 'theme', 'extensions:changed'] as const) {
    api.events.on(name as any, () => { seen[name] = (seen[name] ?? 0) + 1; });
  }
  api.commands.register({
    id: `${api.id}.run`,
    label: 'Run sandbox API probe',
    run: async (input: { t?: number; layerId?: string } = {}) => {
      const out: Record<string, Result> = {};
      const run = async (name: string, fn: () => Promise<Result> | Result) => {
        try { out[name] = await fn(); } catch (error) { out[name] = { ok: false, detail: `threw: ${String((error as Error)?.message ?? error)}` }; }
      };
      const t = input.t ?? 0.5;
      const isPromise = (v: unknown) => !!v && typeof (v as any).then === 'function';
      let snapshotUrl = '';
      await run('project.get Promise of a frozen snapshot', async () => {
        const pending: any = api.project.get();
        const project = await pending;
        return { ok: isPromise(pending) && Object.isFrozen(project) && Array.isArray(project.layers) && project.layers.length > 0, detail: { layers: project?.layers?.length } };
      });
      await run('reactive reads stay synchronous', () => {
        const r = api.project.revision(), time = api.project.time(), playing = api.project.playing(), sel: any = api.project.selection(), tt = api.transport.time();
        return { ok: Number.isInteger(r) && typeof time === 'number' && typeof playing === 'boolean' && !!sel && !isPromise(sel) && typeof tt === 'number', detail: { r, time } };
      });
      await run('project.snapshot from the engine', async () => {
        const pending: any = api.project.snapshot(t, 240);
        snapshotUrl = await pending;
        return { ok: isPromise(pending) && typeof snapshotUrl === 'string' && snapshotUrl.startsWith('data:image/jpeg'), detail: snapshotUrl.length };
      });
      // Without `project:write` (apiVersion 1 and 2 cannot declare it) the
      // writes reject with today's permission error.
      const writes = (api.manifest.permissions ?? []).includes('project:write');
      const refused = async (fn: () => unknown) => {
        try { await fn(); return { ok: false, detail: 'not refused' }; } catch (error) { const m = String((error as Error)?.message ?? error); return { ok: /requires project:write permission/.test(m), detail: m }; }
      };
      await run('project.setTime Promise', async () => {
        if (!writes) return refused(() => api.project.setTime(t));
        const pending: any = api.project.setTime(t);
        await pending;
        return { ok: isPromise(pending) };
      });
      await run('project.apply and undo Promises', async () => {
        if (!writes) return refused(() => api.project.apply({ type: 'add_layer', layerType: 'solid', name: 'Sandbox solid', duration: 1 } as any));
        const before = (await (api.project.get() as any)).layers.length;
        const pending: any = api.project.apply({ type: 'add_layer', layerType: 'solid', name: 'Sandbox solid', duration: 1 } as any);
        const result = await pending;
        const added = (await (api.project.get() as any)).layers.length;
        await (api.project.undo() as any);
        const after = (await (api.project.get() as any)).layers.length;
        return { ok: isPromise(pending) && result?.ok === true && added === before + 1 && after === before, detail: { before, added, after } };
      });
      await run('commands.run Promise', async () => {
        const pending: any = api.commands.run(`${api.id}.echo`, 7);
        return { ok: isPromise(pending) && (await pending) === 7 };
      });
      await run('assets.get Promise of a serializable record', async () => {
        const pending: any = api.assets.get('missing');
        return { ok: isPromise(pending) && (await pending) === undefined };
      });
      await run('storage Promises', async () => {
        const set: any = api.storage.set('probe', { a: 1 });
        await set;
        const got = await (api.storage.get('probe') as any);
        return { ok: isPromise(set) && got?.a === 1 };
      });
      await run('trusted-only namespaces are not available', () => {
        const absent = (fn: () => unknown) => { try { const v = fn(); return v === undefined || v === null; } catch { return true; } };
        const render = absent(() => (api as any).render.gl.bounds);
        const anim = absent(() => (api as any).anim.ev);
        const services = absent(() => (api as any).services.get);
        return { ok: render && anim && services, detail: { render, anim, services } };
      });
      return { apiVersion: api.manifest.apiVersion, out, snapshotUrl, seen: { ...seen } };
    }
  });
  api.commands.register({ id: `${api.id}.echo`, label: 'Echo', run: (value: unknown) => value });
}
