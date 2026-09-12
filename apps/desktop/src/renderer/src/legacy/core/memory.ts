import type { PMRegistry } from '../registry';

const MIB = 1024 * 1024;
const DEFAULT_BUDGETS: Record<string, number> = {
  history: 128 * MIB,
  raster: 96 * MIB,
  textures: 128 * MIB,
  framebuffers: 256 * MIB,
  audio: 128 * MIB,
};

/** One place for every large cache to declare, measure, and release memory. */
export function install(PM: PMRegistry): void {
  const providers = new Map<string, any>();
  const budgets = { ...DEFAULT_BUDGETS };
  const pending = new Map<string, any>();

  const cancelPending = (name: string) => {
    const scheduled = pending.get(name);
    if (!scheduled) return;
    if (scheduled.kind === 'idle') (window as any).cancelIdleCallback?.(scheduled.id);
    else window.clearTimeout?.(scheduled.id);
    pending.delete(name);
  };

  const schedule = (name: string, action: () => void) => {
    if (pending.has(name)) return;
    if (typeof (window as any).requestIdleCallback === 'function') {
      const id = (window as any).requestIdleCallback(action, { timeout: 750 });
      pending.set(name, { kind: 'idle', id });
    } else {
      const id = window.setTimeout(action, 32);
      pending.set(name, { kind: 'timeout', id });
    }
  };

  PM.Memory = {
    budget(name: string) {
      return budgets[name] ?? 64 * MIB;
    },
    setBudget(name: string, bytes: number) {
      if (!Number.isFinite(bytes) || bytes < MIB) return false;
      budgets[name] = Math.floor(bytes);
      cancelPending(name);
      providers.get(name)?.trim?.(budgets[name]);
      return true;
    },
    register(name: string, provider: any) {
      providers.set(name, provider);
      provider?.trim?.(budgets[name] ?? 64 * MIB);
      return () => { cancelPending(name); providers.delete(name); };
    },
    /**
     * Evict outside the render/audio hot path and with hysteresis. This keeps
     * a cache hovering near its limit from destroying and rebuilding the same
     * resource every frame while preserving the declared steady-state bound.
     */
    maintain(name: string, force = false) {
      const provider = providers.get(name);
      if (!provider) return false;
      const budget = budgets[name] ?? 64 * MIB;
      const usage = Math.max(0, Number(provider?.bytes?.()) || 0);
      if (!force && usage <= budget * 1.15) return false;
      schedule(name, () => {
        pending.delete(name);
        const current = Math.max(0, Number(provider?.bytes?.()) || 0);
        if (force || current > budget) provider?.trim?.(Math.floor(budget * 0.9));
      });
      return true;
    },
    pressure(level: 'moderate' | 'critical' = 'moderate') {
      const ratio = level === 'critical' ? 0.25 : 0.6;
      for (const [name, provider] of providers) {
        cancelPending(name);
        provider?.trim?.(Math.floor((budgets[name] || 64 * MIB) * ratio));
      }
    },
    stats() {
      return Object.fromEntries([...providers].map(([name, provider]) => [name, {
        bytes: Math.max(0, Number(provider?.bytes?.()) || 0),
        budget: budgets[name] ?? 0,
        entries: Math.max(0, Number(provider?.entries?.()) || 0),
      }]));
    },
  };

  window.addEventListener?.('memorypressure', (event: any) => {
    PM.Memory.pressure(event?.detail === 'critical' ? 'critical' : 'moderate');
  });
}
