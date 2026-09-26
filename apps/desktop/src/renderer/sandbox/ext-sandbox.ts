import '@powermove/tokens/tokens.css';
import { bootRuntime, bootView, installSandboxRuntime } from './boot';
import type { SandboxInit, SandboxViewInit } from './shim-api';

installSandboxRuntime();
/* The kernel names the role in its one `init` message; the URL's `view`
   parameter must agree, so a runtime document can never be asked to act as a
   view of some other panel or the other way round. */
const viewParam = new URLSearchParams(location.search).get('view');
let initialized = false;
window.addEventListener('message', (event: MessageEvent<(SandboxInit & { t: string; mode?: string }) | (SandboxViewInit & { t: string })>) => {
  if (initialized || event.source !== parent || event.data?.t !== 'init') return;
  const [first, second] = event.ports;
  if (!(first instanceof MessagePort)) return;
  const init = event.data;
  if (init.mode === 'view') {
    if (!(second instanceof MessagePort) || viewParam !== (init as SandboxViewInit).panelId) return;
    initialized = true;
    void bootView(init as SandboxViewInit, first, second);
    return;
  }
  if (viewParam !== null) return;
  initialized = true;
  void bootRuntime(init, first);
});
