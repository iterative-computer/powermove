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

  PM.Memory = {
    budget(name: string) {
      return budgets[name] ?? 64 * MIB;
    },
    setBudget(name: string, bytes: number) {
      if (!Number.isFinite(bytes) || bytes < MIB) return false;
      budgets[name] = Math.floor(bytes);
      providers.get(name)?.trim?.(budgets[name]);
      return true;
    },
    register(name: string, provider: any) {
      providers.set(name, provider);
      provider?.trim?.(budgets[name] ?? 64 * MIB);
      return () => providers.delete(name);
    },
    pressure(level: 'moderate' | 'critical' = 'moderate') {
      const ratio = level === 'critical' ? 0.25 : 0.6;
      for (const [name, provider] of providers) provider?.trim?.(Math.floor((budgets[name] || 64 * MIB) * ratio));
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
