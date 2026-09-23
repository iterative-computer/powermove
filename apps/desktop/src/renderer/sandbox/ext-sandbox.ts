import * as svelte from 'svelte';
import '@powermove/tokens/tokens.css';
// @ts-expect-error -- Svelte's internal client entry has no declaration
import * as svelteInternalClient from 'svelte/internal/client';
import * as svelteStore from 'svelte/store';
import { createRpc, serializeRpcError } from '../../shared/sandbox-rpc';
import { createSandboxAPI, type SandboxInit, type SandboxMirror } from './shim-api';
import { isProperty, canAnimateContent, contentLabel } from '../src/legacy/core/content-properties';
import { structuredProperties, pathTargets } from '../src/legacy/core/vector-paths';
import { validMatteSource, MATTE_MODES } from '../src/legacy/core/matte';
import { expressionDiagnostic, EXPRESSION_NAMES } from '../src/legacy/core/expression';
import { axisContentKey, axisPath, isAxisTag } from '../src/typography/font-catalog';
import { CHANNELS_3D, projectPoint, inversePlane } from '../src/legacy/core/space-3d';
import { propertyShortcuts } from '../src/kernel/property-shortcuts';
import { EDITOR_HELPER_EXPORTS } from '../../shared/extension-runtime';

const pure: Record<string, unknown> = { isProperty, canAnimateContent, contentLabel, structuredProperties, pathTargets, validMatteSource, MATTE_MODES, expressionDiagnostic, EXPRESSION_NAMES, axisContentKey, axisPath, isAxisTag, CHANNELS_3D, projectPoint, inversePlane, propertyShortcuts };
const helpers = Object.fromEntries(EDITOR_HELPER_EXPORTS.map(name => [name, name in pure ? pure[name] : (..._args: unknown[]) => {
  const error = new Error(`${name} requires full access. Use project.apply or commands in a sandboxed extension.`);
  error.name = 'PermissionError'; (error as Error & { code: string }).code = 'full-access'; throw error;
}]));
(globalThis as any).__powermove_runtime = {
  svelte, 'svelte/internal/client': svelteInternalClient, 'svelte/store': svelteStore,
  'svelte/internal/disclose-version': {}, powermove: helpers
};
let initialized = false;
window.addEventListener('message', async (event: MessageEvent<SandboxInit & { t: string; port: MessagePort }>) => {
  if (initialized || event.source !== parent || event.data?.t !== 'init' || !(event.ports[0] instanceof MessagePort)) return;
  initialized = true;
  const init = event.data;
  let control: { update(next: SandboxMirror): void; dispose(): void; ready(): Promise<unknown> } | undefined;
  const live = createRpc(event.ports[0], {
    mirror: (snapshot: SandboxMirror) => control?.update(snapshot),
    theme: (theme: SandboxInit['theme']) => applyTheme(theme),
    dispose: () => control?.dispose()
  });
  const liveApi = createSandboxAPI(live, init);
  control = (liveApi as unknown as { __sandbox: typeof control }).__sandbox;
  applyTheme(init.theme);
  const runtimeError = (error: unknown) => live.notify('runtime-error', serializeRpcError(error));
  window.addEventListener('error', event => runtimeError(event.error ?? event.message));
  window.addEventListener('unhandledrejection', event => runtimeError(event.reason));
  window.addEventListener('securitypolicyviolation', event => live.notify('csp-violation', { directive: event.violatedDirective, blockedURI: event.blockedURI }));
  try {
    const module = await import(/* @vite-ignore */ init.bundleUrl);
    if (typeof module.default !== 'function') throw new Error('entry module must export default activate(api)');
    const addStyle = (css: string): void => {
      const style = document.createElement('style'); style.textContent = css; document.head.append(style);
      liveApi.onDispose(() => style.remove());
    };
    if (module.__powermoveAcquireStyles) liveApi.onDispose(module.__powermoveAcquireStyles(addStyle));
    else for (const css of module.__powermoveStyles ?? []) addStyle(css);
    const result = await module.default(liveApi);
    await control?.ready();
    if (result?.dispose) liveApi.onDispose(() => result.dispose());
    live.notify('activated');
  } catch (error) { live.notify('activation-error', serializeRpcError(error)); }
}, { once: true });
function applyTheme(theme: SandboxInit['theme']): void {
  document.documentElement.dataset.theme = theme.scheme;
  for (const [key, value] of Object.entries(theme.tokens ?? {})) if (key.startsWith('--')) document.documentElement.style.setProperty(key, value);
}
