/*
 * Boot code shared by the extension sandbox document (ext-sandbox.html).
 *
 * One document, two roles, picked by the kernel's `init` message:
 *   - runtime: the extension's one long-lived iframe; runs `activate(api)` and
 *     forwards registrations to the kernel (X2);
 *   - view: one iframe per open panel; imports the same bundle, replays
 *     `activate` against a view-mode API (registrations stay local, see
 *     shim-api.ts), then mounts the recorded panel definition into its body.
 *
 * Kept free of top-level side effects so tests can drive both roles in a
 * happy-dom document with MessageChannel ports.
 */
import * as svelte from 'svelte';
// @ts-expect-error -- Svelte's internal client entry has no declaration
import * as svelteInternalClient from 'svelte/internal/client';
import * as svelteStore from 'svelte/store';
import type { Component } from 'svelte';
import { createRpc, serializeRpcError } from '../../shared/sandbox-rpc';
import { createSandboxAPI, sandboxControl, sandboxReporter, type SandboxReporter, type SandboxInit, type SandboxKey, type SandboxKeyEvent, type SandboxMirror, type SandboxViewInit } from './shim-api';
import { isProperty, canAnimateContent, contentLabel } from '../src/legacy/core/content-properties';
import { structuredProperties, pathTargets } from '../src/legacy/core/vector-paths';
import { validMatteSource, MATTE_MODES } from '../src/legacy/core/matte';
import { expressionDiagnostic, EXPRESSION_NAMES } from '../src/legacy/core/expression';
import { axisContentKey, axisPath, isAxisTag } from '../src/typography/font-catalog';
import { CHANNELS_3D, projectPoint, inversePlane } from '../src/legacy/core/space-3d';
import { propertyShortcuts } from '../src/kernel/property-shortcuts';
import { chordMatches, chordOfEvent, isFieldTarget } from '../src/kernel/keychord';
import { EDITOR_HELPER_EXPORTS } from '../../shared/extension-runtime';
import type { PowermoveAPI } from '../src/kernel/api';

type ExtensionModule = {
  default?: (api: PowermoveAPI) => unknown;
  __powermoveAcquireStyles?: (add: (css: string) => void) => () => void;
  __powermoveStyles?: string[];
};
export type BundleImporter = (url: string) => Promise<ExtensionModule>;
const importBundle: BundleImporter = url => import(/* @vite-ignore */ url);

/* The editor helpers are module imports, not `api` members, so the document's
   role (runtime or view) hands them its reporter once it has a port. */
let helperReport: SandboxReporter | null = null;

/** Fill the runtime table the compiled bundle resolves `svelte`/`powermove` against. */
export function installSandboxRuntime(): void {
  const pure: Record<string, unknown> = { isProperty, canAnimateContent, contentLabel, structuredProperties, pathTargets, validMatteSource, MATTE_MODES, expressionDiagnostic, EXPRESSION_NAMES, axisContentKey, axisPath, isAxisTag, CHANNELS_3D, projectPoint, inversePlane, propertyShortcuts };
  const helpers = Object.fromEntries(EDITOR_HELPER_EXPORTS.map(name => [name, name in pure ? pure[name] : (..._args: unknown[]) => {
    helperReport?.('permission', `powermove.${name}`);
    const error = new Error(`${name} requires full access. Use project.apply or commands in a sandboxed extension.`);
    error.name = 'PermissionError'; (error as Error & { code: string }).code = 'full-access'; throw error;
  }]));
  Reflect.set(globalThis, '__powermove_runtime', {
    svelte, 'svelte/internal/client': svelteInternalClient, 'svelte/store': svelteStore,
    'svelte/internal/disclose-version': {}, powermove: helpers
  });
}

/** Applies the host's theme; removes whatever the previous push set. */
export function themeApplier(doc: Document = document): (theme: SandboxInit['theme']) => void {
  let keys: string[] = [];
  return theme => {
    const root = doc.documentElement;
    root.dataset.theme = theme.scheme;
    for (const key of keys) root.style.removeProperty(key);
    keys = [];
    for (const [key, value] of Object.entries(theme.tokens ?? {})) {
      if (!key.startsWith('--')) continue;
      root.style.setProperty(key, String(value));
      keys.push(key);
    }
  };
}

function attachStyles(module: ExtensionModule, api: PowermoveAPI, doc: Document): void {
  const addStyle = (css: string): void => {
    const style = doc.createElement('style'); style.textContent = css; doc.head.append(style);
    api.onDispose(() => style.remove());
  };
  if (module.__powermoveAcquireStyles) api.onDispose(module.__powermoveAcquireStyles(addStyle));
  else for (const css of module.__powermoveStyles ?? []) addStyle(css);
}

/** Runtime role: activate the extension and forward its registrations. */
export async function bootRuntime(init: SandboxInit, port: MessagePort, load: BundleImporter = importBundle): Promise<void> {
  const apply = themeApplier();
  let control: ReturnType<typeof sandboxControl> | undefined;
  const live = createRpc(port, {
    mirror: (snapshot: SandboxMirror) => control?.update(snapshot),
    theme: (theme: SandboxInit['theme']) => apply(theme),
    mountPanel: (panelId: string, token: string, viewPort: MessagePort) => control?.mountPanel(panelId, token, viewPort),
    unmountPanel: (token: string) => control?.unmountPanel(token),
    dispose: () => control?.dispose()
  }, 10_000, { maxMirrorBytes: 16 * 1024 * 1024 + 8192, maxHandles: 1000 });
  const liveApi = createSandboxAPI(live, init);
  control = sandboxControl(liveApi);
  helperReport = sandboxReporter(live);
  apply(init.theme);
  const runtimeError = (error: unknown) => live.notify('runtime-error', serializeRpcError(error));
  window.addEventListener('error', event => runtimeError(event.error ?? event.message));
  window.addEventListener('unhandledrejection', event => runtimeError(event.reason));
  window.addEventListener('securitypolicyviolation', event => live.notify('csp-violation', { directive: event.violatedDirective, blockedURI: event.blockedURI }));
  try {
    const module = await load(init.bundleUrl);
    if (typeof module.default !== 'function') throw new Error('entry module must export default activate(api)');
    attachStyles(module, liveApi, document);
    const result = await module.default(liveApi) as { dispose?: () => void } | undefined;
    await control.ready();
    const dispose = result?.dispose;
    if (dispose) liveApi.onDispose(() => dispose());
    live.notify('activated');
  } catch (error) { live.notify('activation-error', serializeRpcError(error)); }
}

/* Panel documents fill the host's panel body and inherit the app's surface:
   transparent so the panel's own background shows, the app's type ramp. */
const VIEW_CSS = `html,body{margin:0;padding:0;background:transparent}
html{overflow:auto;scrollbar-width:none}html::-webkit-scrollbar{display:none}
html[data-noscroll]{overflow:hidden}
body{min-height:100%;color:var(--tx);font-family:var(--f-ui);font-size:var(--fs-md);-webkit-font-smoothing:antialiased;text-rendering:optimizeLegibility;-webkit-user-select:none;user-select:none;cursor:default}
input,textarea,[contenteditable]:not([contenteditable="false"]){-webkit-user-select:text;user-select:text}
.pm-view-error{padding:14px 12px;color:var(--tx-3);font-size:var(--fs-sm);line-height:1.45;text-wrap:pretty}`;

/** Decide which keydowns the host needs: those matching one of its bindings, plus Escape. */
export function keyToForward(event: KeyboardEvent, keys: readonly SandboxKey[]): { forward: SandboxKeyEvent; prevent: boolean } | null {
  if (event.defaultPrevented) return null; // the panel handled it itself
  const chord = chordOfEvent(event);
  if (!chord) return null;
  const target = event.target as Element | null;
  // Same rules as the host listener: Enter activates a focused control, and a text selection owns Copy.
  if (chord === 'enter' && target?.closest?.('button,a[href],select,[role="button"],[role="radio"]')) return null;
  if ((chord === 'cmd+c' || chord === 'ctrl+c') && String(globalThis.getSelection?.() ?? '')) return null;
  const field = isFieldTarget(target) || (event.composedPath?.() ?? []).some(isFieldTarget);
  const match = keys.find(key => (!field || key.inFields) && (!event.repeat || key.repeat) && chordMatches(key.chord, chord, key.looseModifiers));
  if (!match && chord !== 'escape' && chord !== 'tab' && chord !== 'shift+tab') return null;
  return {
    forward: { key: event.key, code: event.code, metaKey: event.metaKey, ctrlKey: event.ctrlKey, altKey: event.altKey, shiftKey: event.shiftKey, repeat: event.repeat, field },
    /* The host cancels ordinary bindings before running them; mirror that
       here so Space or an arrow does not also scroll the panel. Field-aware
       bindings leave the native field behaviour alone. */
    prevent: !!match && !field && !match.inFields
  };
}

export interface SandboxView { dispose(): void }

/** View role: mount one panel of the extension into this document. */
export async function bootView(init: SandboxViewInit, kernelPort: MessagePort, runtimePort: MessagePort, options: { load?: BundleImporter; target?: HTMLElement } = {}): Promise<SandboxView> {
  const load = options.load ?? importBundle;
  const target = options.target ?? document.body;
  const doc = target.ownerDocument;
  const win = doc.defaultView ?? window;
  const root = doc.documentElement;
  const apply = themeApplier(doc);
  let keys = init.keys ?? [];
  let control: ReturnType<typeof sandboxControl> | undefined;
  let torn = false;
  const setSize = (size: { width: number; height: number } | undefined): void => {
    if (!size) return;
    root.style.width = `${Math.max(0, Math.round(size.width))}px`;
    root.style.height = `${Math.max(0, Math.round(size.height))}px`;
  };
  const teardown = (): void => {
    if (torn) return; torn = true;
    win.removeEventListener('keydown', onKey);
    doc.removeEventListener('focusin', onFocus, true);
    doc.removeEventListener('pointerdown', onPointer, true);
    control?.dispose();
    runtime.close();
  };
  const kernel = createRpc(kernelPort, {
    mirror: (snapshot: SandboxMirror) => control?.update(snapshot),
    theme: (theme: SandboxInit['theme']) => apply(theme),
    size: setSize,
    keys: (next: SandboxKey[]) => { keys = next; },
    dispose: teardown
  }, 10_000, { maxMirrorBytes: 16 * 1024 * 1024 + 8192, maxHandles: 20 });
  const runtime = createRpc(runtimePort, {});
  helperReport = sandboxReporter(kernel);
  const onKey = (event: KeyboardEvent): void => {
    const decision = keyToForward(event, keys);
    if (!decision) return;
    if (decision.prevent) event.preventDefault();
    kernel.notify('key', decision.forward);
  };
  const onFocus = (): void => kernel.notify('focus', { field: isFieldTarget(doc.activeElement) });
  /* A press inside the panel never reaches the app's document, so menus and
     popovers open there would not hear the outside click. */
  const onPointer = (event: PointerEvent): void => kernel.notify('pointer', { button: event.button, x: event.clientX, y: event.clientY });
  win.addEventListener('keydown', onKey);
  doc.addEventListener('focusin', onFocus, true);
  doc.addEventListener('pointerdown', onPointer, true);
  win.addEventListener('error', event => kernel.notify('runtime-error', serializeRpcError(event.error ?? event.message)));
  win.addEventListener('unhandledrejection', event => kernel.notify('runtime-error', serializeRpcError(event.reason)));
  win.addEventListener('securitypolicyviolation', event => kernel.notify('csp-violation', { directive: event.violatedDirective, blockedURI: event.blockedURI }));

  const base = doc.createElement('style');
  base.textContent = VIEW_CSS;
  doc.head.append(base);
  if (init.noscroll) root.dataset.noscroll = '';
  apply(init.theme);
  setSize(init.size);

  try {
    const info = await runtime.call('definition', init.panelId);
    if (!info) throw new Error(`Panel "${init.panelId}" is no longer registered.`);
    const api = createSandboxAPI(kernel, init, 'view');
    control = sandboxControl(api);
    const module = await load(init.bundleUrl);
    if (typeof module.default !== 'function') throw new Error('entry module must export default activate(api)');
    attachStyles(module, api, doc);
    control.setQuiet(true);
    try {
      const result = await module.default(api) as { dispose?: () => void } | undefined;
      const dispose = result?.dispose;
      if (dispose) api.onDispose(() => dispose());
    } finally { control.setQuiet(false); }
    const def = control.panel(init.panelId);
    if (!def) throw new Error(`activate() did not register panel "${init.panelId}" in this view.`);
    const props = { panelId: init.panelId, spec: init.spec ?? {}, api };
    if (def.component) {
      const instance = svelte.mount(def.component as Component<typeof props>, { target, props });
      svelte.flushSync();
      api.onDispose(() => void svelte.unmount(instance));
    } else {
      const cleanup = def.build(target, { spec: props.spec });
      if (typeof cleanup === 'function') api.onDispose(cleanup);
    }
    kernel.notify('mounted');
  } catch (error) {
    const message = doc.createElement('div');
    message.className = 'pm-view-error';
    message.textContent = `This panel couldn’t load. ${error instanceof Error ? error.message : String(error)}`;
    target.replaceChildren(message);
    kernel.notify('view-error', serializeRpcError(error));
  }
  return { dispose: teardown };
}
