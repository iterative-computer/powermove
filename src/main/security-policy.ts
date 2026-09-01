/** The privileged editor never evaluates project-authored JavaScript. */
export const CONTENT_SECURITY_POLICY =
  "default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' blob: data:; media-src 'self' blob:; font-src 'self'; connect-src 'self' blob:; worker-src 'self' blob:; frame-src 'self' about: blob:";

/** Generated scripts retain eval only inside the opaque, no-network sandbox. */
export const SANDBOX_CONTENT_SECURITY_POLICY =
  "default-src 'none'; script-src app://powermove/host/sandbox.js 'unsafe-eval'; worker-src blob:; connect-src 'none'";
