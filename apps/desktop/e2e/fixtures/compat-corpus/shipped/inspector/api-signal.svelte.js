/* A tiny rune-backed invalidation signal for API event adapters. */
export function createInspectorSignal() {
  let version = $state(0);
  return {
    get version() { return version; },
    bump() { version++; }
  };
}
