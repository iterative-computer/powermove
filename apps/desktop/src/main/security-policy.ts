import { isSandboxHost, sandboxDocumentId, sandboxHost } from '../shared/sandbox-origin';

/**
 * Trusted extensions can use the network; project-authored scripts run in a
 * separate sandbox. `app:` frames are Store extensions, each on its own host
 * (shared/sandbox-origin.ts); the navigation guard narrows it to their documents.
 * `'wasm-unsafe-eval'` lets the editor compile its own WebAssembly (pm-wasm,
 * the document mirror; docs/rust-engine.md section 26, S5): it permits
 * `WebAssembly.compile` and `instantiate` only, never string evaluation
 * (`eval` and `new Function` stay blocked).
 */
export const CONTENT_SECURITY_POLICY =
  "default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self' 'unsafe-inline'; img-src 'self' blob: data: http: https:; media-src 'self' blob:; font-src 'self'; connect-src 'self' blob: http: https: ws: wss:; worker-src 'self' blob:; frame-src 'self' about: blob: app:";

/** Generated scripts retain eval only inside the opaque, no-network sandbox. */
export const SANDBOX_CONTENT_SECURITY_POLICY =
  "default-src 'none'; script-src app://powermove/host/sandbox.js 'unsafe-eval'; worker-src blob:; connect-src 'none'";

/**
 * The document policy is derived from the main-owned manifest only. `origin`
 * is the extension's own sandbox origin (`app://<sandboxHost(id)>`, or the
 * browser host's single origin). `dev` is the Vite dev-server origin: in development the sandbox document's
 * bootstrap is served by Vite (with its HMR client), so script and connect
 * sources include it. Production builds never pass it.
 */
export function extensionSandboxCsp(id: string, permissions: readonly string[], origin: string, dev?: string): string {
  const network = permissions.includes('network');
  const remote = network ? ' https:' : '';
  const devScript = dev ? ` ${dev}/` : '';
  const devConnect = dev ? ` ${dev} ${dev.replace(/^http/, 'ws')}` : '';
  /* data: and blob: never leave the machine (an opaque origin cannot read
     another frame's blobs), so `fetch(canvas.toDataURL())`, object URLs and
     inlined wasm or JSON work without network. */
  const connect = `data: blob:${network ? ' https: wss:' : ''}${devConnect}`;
  return [
    "default-src 'none'",
    `script-src ${origin}/host/ ${origin}/ext/${id}/ 'wasm-unsafe-eval'${devScript}`,
    // Bundled fonts are data: URLs; remote fonts and stylesheets need network. CSS cannot run script.
    `style-src 'unsafe-inline' ${origin}/${remote}${devScript}`,
    `font-src ${origin}/ data: blob:${remote}`,
    `img-src data: blob:${remote}`,
    `media-src data: blob:${remote}`,
    `connect-src ${connect}`,
    'worker-src blob:', "frame-src 'none'", "base-uri 'none'", "form-action 'none'"
  ].join('; ');
}

const hostOf = (url: string | null | undefined): string => { try { return url ? new URL(url).host : ''; } catch { return ''; } };

/**
 * A sub-frame may load a sandbox document only on its own extension's host,
 * and a frame that is (or was asked to navigate by) one extension's sandbox
 * never becomes another's. `from` holds the frame's current URL and the
 * initiator's, when known.
 */
export function sandboxFrameNavigationAllowed(target: string, from: ReadonlyArray<string | null | undefined>): boolean {
  if (sandboxDocumentId(target) === null) return false;
  const host = hostOf(target);
  return from.every(url => { const other = hostOf(url); return !isSandboxHost(other) || other === host; });
}

/**
 * The one extension among `ids` whose sandbox host is `host`, else null. Two
 * ids on one host would share a process, so a collision (infeasible with
 * SHA-256, but main never trusts that) owns nothing: main then serves and
 * kills nothing there. `hostFor` is only replaced by tests.
 */
export function sandboxHostOwner(host: string, ids: Iterable<string>, hostFor: (id: string) => string = sandboxHost): string | null {
  let owner: string | null = null;
  for (const id of new Set(ids)) {
    if (hostFor(id) !== host) continue;
    if (owner !== null) return null;
    owner = id;
  }
  return owner;
}

export interface ContentsFrames { mainFramePid: number; frames: ReadonlyArray<{ url: string; pid: number }> }

/**
 * The OS processes that belong to one sandbox host alone: every frame on the
 * pid, in every WebContents, is on `host`, and no WebContents' main frame
 * (an editor, a popout, onboarding) runs there. Anything shared is left alone.
 */
export function sandboxProcessesToKill(host: string, contents: readonly ContentsFrames[]): number[] {
  if (!isSandboxHost(host)) return [];
  const hosts = new Map<number, Set<string>>();
  const mainPids = new Set(contents.map(entry => entry.mainFramePid));
  for (const { frames } of contents) for (const { url, pid } of frames) {
    if (!(pid > 0)) continue;
    const set = hosts.get(pid) ?? new Set<string>(); set.add(hostOf(url)); hosts.set(pid, set);
  }
  return [...hosts].filter(([pid, set]) => !mainPids.has(pid) && set.size === 1 && set.has(host)).map(([pid]) => pid);
}
