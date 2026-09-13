type WarmupState = {
  key: string;
  project: any;
  time: number;
  blocked: boolean;
};

/** Prepare imminent sources in short idle slices. The queue never renders a
 * future frame or takes ownership of the visible canvas or media decoders. */
export function createPreviewWarmup(
  state: () => WarmupState,
  prepare: (layer: any, time: number) => void,
  schedule: (run: (deadline: { timeRemaining(): number }) => void) => void,
  now = () => performance.now(),
) {
  let project: any, key = '', lastTime = -Infinity, lastScan = -Infinity, pending = false;
  let queue: any[] = [], attempted = new Set<any>();
  const refresh = (current: WarmupState) => {
    if (project !== current.project || key !== current.key || current.time < lastTime) {
      project = current.project; key = current.key; attempted = new Set(); queue = []; lastScan = -Infinity;
    }
    lastTime = current.time;
    if (queue.length || current.time < lastScan + .25) return;
    lastScan = current.time;
    queue = (project?.layers || []).filter((layer: any) =>
      (layer.type === 'shape' || layer.type === 'text') && !attempted.has(layer)
      && layer.from > current.time && layer.from <= current.time + 2
      && layer.on !== false && layer.dur > 0,
    ).sort((a: any, b: any) => a.from - b.from);
  };
  const request = () => {
    if (pending) return;
    const current = state();
    if (current.blocked) return;
    refresh(current);
    if (!queue.length) return;
    pending = true;
    const requestedProject = project, requestedKey = key;
    schedule(deadline => {
      pending = false;
      const current = state();
      if (current.blocked || current.time < lastTime || requestedProject !== current.project || requestedKey !== current.key) {
        queue = [];
        // A paused redraw may already have requested the new key while this
        // slice was pending. Requeue it now; there may be no subsequent frame.
        request(); return;
      }
      const start = now();
      while (queue.length && now() - start < 2 && deadline.timeRemaining() > 1) {
        const layer = queue.shift();
        attempted.add(layer);
        if (layer.from > current.time && layer.from <= current.time + 2) {
          try { prepare(layer, layer.from); }
          catch { /* The visible render remains responsible for source errors. */ }
        }
      }
      if (queue.length) request();
    });
  };
  return request;
}
