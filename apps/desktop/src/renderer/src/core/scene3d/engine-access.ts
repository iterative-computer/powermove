/** Flag-on geometry is always engine-backed, including while startup or an
 * engine restart has not made the query API available yet. */
export function engineGeometry(PM: any, methods: readonly string[]): any | null {
  if (!PM?.EngineDoc?.enabled) return null;
  const api = PM.EngineDoc.api;
  if (api && methods.every(name => typeof api[name] === 'function')) return api;
  const error = new Error(`The native 3D geometry API is unavailable (${methods.join(', ')})`);
  error.name = 'EngineFailure';
  PM.toast?.(error.message, 2200, { key: 'scene3d-engine-unavailable', error: true });
  throw error;
}
