export type IdlePreload = {
  start(): Promise<any>;
  cancel(): void;
};

/** Load an optional feature after boot, or immediately when first requested. */
export function idlePreload(host: any, loader: () => Promise<any>, timeout = 1_500): IdlePreload {
  let promise: Promise<any> | null = null;
  let scheduled: { kind: 'idle' | 'timeout'; id: any } | null = null;
  const clear = () => {
    if (!scheduled) return;
    if (scheduled.kind === 'idle') host.cancelIdleCallback?.(scheduled.id);
    else (host.clearTimeout?.bind(host) || globalThis.clearTimeout)(scheduled.id);
    scheduled = null;
  };
  const start = () => {
    clear();
    promise ||= loader();
    return promise;
  };
  if (typeof host.requestIdleCallback === 'function') {
    const id = host.requestIdleCallback(() => { void start(); }, { timeout });
    scheduled = { kind: 'idle', id };
  } else {
    const set = host.setTimeout?.bind(host) || globalThis.setTimeout;
    const id = set(() => { void start(); }, Math.min(timeout, 1_000));
    scheduled = { kind: 'timeout', id };
  }
  return { start, cancel: clear };
}
