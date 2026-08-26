/* Reactive bridge for imperative engines that invoke PM.Inspector.refresh(). */
class InspectorRefresh {
  version = $state(0);

  bump(): void {
    this.version++;
  }
}

export const inspectorRefresh = new InspectorRefresh();

let registry: Record<string, any> | undefined;

export function configureInspectorRegistry(PM: Record<string, any>): void {
  registry = PM;
}

export function getInspectorRegistry(): Record<string, any> {
  const PM = registry ?? (typeof window !== 'undefined' ? (window as any).PM : undefined);
  if (!PM) throw new Error('InspectorPanel requires a configured PM registry');
  return PM;
}
