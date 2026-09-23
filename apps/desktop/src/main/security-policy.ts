/** Trusted extensions can use the network; project-authored scripts run in a separate sandbox. */
export const CONTENT_SECURITY_POLICY =
  "default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' blob: data:; media-src 'self' blob:; font-src 'self'; connect-src 'self' blob: http: https: ws: wss:; worker-src 'self' blob:; frame-src 'self' about: blob:";

/** Generated scripts retain eval only inside the opaque, no-network sandbox. */
export const SANDBOX_CONTENT_SECURITY_POLICY =
  "default-src 'none'; script-src app://powermove/host/sandbox.js 'unsafe-eval'; worker-src blob:; connect-src 'none'";

/** The document policy is derived from the main-owned manifest only. */
export function extensionSandboxCsp(id: string, permissions: readonly string[], origin = 'app://powermove'): string {
  const network = permissions.includes('network');
  const remote = network ? ' https:' : '';
  return [
    "default-src 'none'",
    `script-src ${origin}/host/ ${origin}/ext/${id}/ 'wasm-unsafe-eval'`,
    `style-src 'unsafe-inline' ${origin}/`,
    `font-src ${origin}/`,
    `img-src data: blob:${remote}`,
    `media-src blob:${remote}`,
    `connect-src ${network ? 'https: wss:' : "'none'"}`,
    'worker-src blob:', "frame-src 'none'", "base-uri 'none'", "form-action 'none'"
  ].join('; ');
}
