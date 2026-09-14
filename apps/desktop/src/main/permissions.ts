import type { Session, WebContents } from 'electron';

// Numeric scrubbing needs pointer lock to continue beyond display edges.
const GRANTED_PERMISSIONS = new Set(['local-fonts', 'pointerLock']);

export function installPermissionHandlers(
  session: Pick<Session, 'setPermissionRequestHandler' | 'setPermissionCheckHandler'>,
  isAppMainFrame: (contents: WebContents | null, url: string | undefined) => boolean,
): void {
  session.setPermissionRequestHandler((webContents, permission, callback, details) => {
    callback(GRANTED_PERMISSIONS.has(permission)
      && (permission !== 'pointerLock' || details.isMainFrame)
      && isAppMainFrame(webContents, details.requestingUrl));
  });
  session.setPermissionCheckHandler((webContents, permission, requestingOrigin, details) => {
    return GRANTED_PERMISSIONS.has(permission)
      && (permission !== 'pointerLock' || details.isMainFrame)
      && isAppMainFrame(webContents, requestingOrigin);
  });
}
