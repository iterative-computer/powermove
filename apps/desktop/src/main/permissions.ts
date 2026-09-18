import path from 'node:path';
import type { Session, WebContents } from 'electron';

// Numeric scrubbing needs pointer lock to continue beyond display edges.
const GRANTED_PERMISSIONS = new Set(['local-fonts', 'pointerLock']);

export function installPermissionHandlers(
  session: Pick<Session, 'setPermissionRequestHandler' | 'setPermissionCheckHandler' | 'on'>,
  isAppMainFrame: (contents: WebContents | null, url: string | undefined) => boolean,
): void {
  // A directory picker supplies the handle; remember only paths selected by
  // the trusted app document, then permit files created beneath that handle.
  type Selection = { documentUrl: string; directories: Set<string>; clear: () => void };
  const selections = new Map<WebContents, Selection>();
  const withinSelectedDirectory = (directories: Set<string> | undefined, filePath: string): boolean =>
    path.isAbsolute(filePath) && [...(directories ?? [])].some(directory => {
      const relative = path.relative(directory, filePath);
      return relative === '' || (relative !== '..' && !relative.startsWith(`..${path.sep}`) && !path.isAbsolute(relative));
    });
  const isAppDocument = (contents: WebContents | null, url: string | undefined): contents is WebContents =>
    !!contents && !contents.isDestroyed() && !!url && typeof contents.getURL === 'function'
      && contents.getURL() === url && isAppMainFrame(contents, url);
  const sameOrigin = (documentUrl: string, requestingOrigin: string): boolean => {
    try {
      const document = new URL(documentUrl);
      const origin = new URL(requestingOrigin);
      return document.protocol === origin.protocol && document.host === origin.host;
    } catch { return false; }
  };
  const rememberDirectory = (contents: WebContents, documentUrl: string, directory: string): void => {
    let selection = selections.get(contents);
    if (selection?.documentUrl !== documentUrl) {
      selection?.clear();
      const clear = () => {
        selections.delete(contents);
        contents.removeListener('did-navigate', clear);
        contents.removeListener('destroyed', clear);
      };
      selection = { documentUrl, directories: new Set(), clear };
      selections.set(contents, selection);
      contents.on('did-navigate', clear);
      contents.on('destroyed', clear);
    }
    selection.directories.add(directory);
  };

  // Chromium waits for this callback before resolving a picker aimed at a
  // restricted location. Let it reject so the user can choose another folder.
  session.on('file-system-access-restricted', (_event, _details, callback) => callback('deny'));

  session.setPermissionRequestHandler((webContents, permission, callback, details) => {
    if (permission === 'fileSystem') {
      const request = details as Electron.FilesystemPermissionRequest;
      const filePath = request.filePath;
      const trusted = request.isMainFrame && isAppDocument(webContents, request.requestingUrl);
      const selectedDirectory = trusted && request.isDirectory === true && !!filePath && path.isAbsolute(filePath);
      const selection = selections.get(webContents);
      const previouslySelected = trusted && !!filePath && selection?.documentUrl === request.requestingUrl
        && withinSelectedDirectory(selection.directories, filePath);
      if (selectedDirectory) rememberDirectory(webContents, request.requestingUrl, filePath);
      callback(!!(selectedDirectory || previouslySelected));
      return;
    }
    callback(GRANTED_PERMISSIONS.has(permission)
      && (permission !== 'pointerLock' || details.isMainFrame)
      && isAppMainFrame(webContents, details.requestingUrl));
  });
  session.setPermissionCheckHandler((webContents, permission, requestingOrigin, details) => {
    if (permission === 'fileSystem') {
      // Chromium reports isMainFrame=false for this permission even for the
      // main document, and may omit requestingUrl. Match the owning document
      // and the selected path instead.
      const documentUrl = details.requestingUrl || webContents?.getURL();
      if (!details.filePath || (details.embeddingOrigin && !sameOrigin(details.embeddingOrigin, requestingOrigin))) return false;
      if (webContents) {
        const selection = selections.get(webContents);
        return isAppDocument(webContents, documentUrl)
          && sameOrigin(documentUrl!, requestingOrigin)
          && selection?.documentUrl === documentUrl
          && withinSelectedDirectory(selection?.directories, details.filePath);
      }
      // Electron's origin-only filesystem check has no frame or WebContents.
      // Require a live trusted owner that selected this exact directory tree.
      return [...selections].some(([owner, selection]) =>
        !owner.isDestroyed() && isAppDocument(owner, selection.documentUrl)
        && sameOrigin(selection.documentUrl, requestingOrigin)
        && withinSelectedDirectory(selection.directories, details.filePath!));
    }
    return GRANTED_PERMISSIONS.has(permission)
      && (permission !== 'pointerLock' || details.isMainFrame)
      && isAppMainFrame(webContents, requestingOrigin);
  });
}
