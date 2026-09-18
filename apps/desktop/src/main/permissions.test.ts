import { EventEmitter } from 'node:events';
import { describe, expect, it, vi } from 'vitest';
import type { Session, WebContents } from 'electron';
import { installPermissionHandlers } from './permissions';

function handlers(trusted = true) {
  const on = vi.fn();
  const session = {
    setPermissionRequestHandler: vi.fn<Session['setPermissionRequestHandler']>(),
    setPermissionCheckHandler: vi.fn<Session['setPermissionCheckHandler']>(),
    on: on as unknown as Session['on'],
  };
  const isAppMainFrame = vi.fn(() => trusted);
  installPermissionHandlers(session, isAppMainFrame);
  return {
    request: session.setPermissionRequestHandler.mock.calls[0]![0]!,
    check: session.setPermissionCheckHandler.mock.calls[0]![0]!,
    restricted: on.mock.calls.find(([name]) => name === 'file-system-access-restricted')?.[1],
    isAppMainFrame,
  };
}

const contents = {} as WebContents;
const origin = 'app://powermove';
const details = { isMainFrame: true, requestingUrl: `${origin}/index.html` };
function appContents(url: string): WebContents {
  return Object.assign(new EventEmitter(), { getURL: () => url, isDestroyed: () => false }) as unknown as WebContents;
}

describe('desktop permissions', () => {
  it('allows numeric scrubbing to acquire pointer lock in the app', () => {
    const { request, check, isAppMainFrame } = handlers();
    const callback = vi.fn();
    request(contents, 'pointerLock', callback, details);
    expect(callback).toHaveBeenCalledWith(true);
    expect(isAppMainFrame).toHaveBeenLastCalledWith(contents, details.requestingUrl);
    expect(check(contents, 'pointerLock', origin, details)).toBe(true);
    expect(isAppMainFrame).toHaveBeenLastCalledWith(contents, origin);
  });

  it('denies pointer lock to untrusted documents and subframes', () => {
    for (const [trusted, isMainFrame] of [[false, true], [true, false]] as const) {
      const { request, check } = handlers(trusted);
      const callback = vi.fn();
      request(contents, 'pointerLock', callback, { ...details, isMainFrame });
      expect(callback).toHaveBeenCalledWith(false);
      expect(check(contents, 'pointerLock', origin, { ...details, isMainFrame })).toBe(false);
    }
  });

  it('continues denying unrelated permissions', () => {
    const { request, check } = handlers();
    const callback = vi.fn();
    request(contents, 'media', callback, details);
    expect(callback).toHaveBeenCalledWith(false);
    expect(check(contents, 'media', origin, details)).toBe(false);
  });

  it('permits filesystem access only beneath a directory selected by the app document', () => {
    const { request, check } = handlers();
    const owner = appContents(details.requestingUrl);
    const otherContents = appContents(details.requestingUrl);
    const selected = '/tmp/powermove-export';
    const fileDetails = { ...details, filePath: selected, isDirectory: true, fileAccessType: 'writable' as const };
    const checked = { ...fileDetails, isMainFrame: false };
    const callback = vi.fn();

    expect(check(owner, 'fileSystem', origin, checked)).toBe(false);
    request(owner, 'fileSystem', callback, fileDetails);
    expect(callback).toHaveBeenCalledWith(true);
    expect(check(owner, 'fileSystem', origin, checked)).toBe(true);
    expect(check(owner, 'fileSystem', origin, { ...checked, filePath: `${selected}/frame.png`, isDirectory: false })).toBe(true);
    expect(check(owner, 'fileSystem', origin, { ...checked, filePath: '/tmp/powermove-export-other/frame.png' })).toBe(false);
    expect(check(otherContents, 'fileSystem', origin, checked)).toBe(false);
    expect(check(owner, 'fileSystem', 'https://example.com', checked)).toBe(false);
    expect(check(owner, 'fileSystem', origin, { ...checked, embeddingOrigin: 'https://example.com' })).toBe(false);
  });

  it('accepts Electron’s filesystem check without requestingUrl after a trusted picker request', () => {
    const { request, check } = handlers();
    const documentUrl = 'app://powermove/';
    const owner = appContents(documentUrl);
    const directory = '/Users/judekim/Downloads/Powermove Release QA';
    const callback = vi.fn();
    request(owner, 'fileSystem', callback, {
      isMainFrame: true, requestingUrl: documentUrl,
      fileAccessType: 'writable', filePath: directory, isDirectory: true,
    });
    expect(callback).toHaveBeenCalledWith(true);

    const nativeCheck = { isMainFrame: false, fileAccessType: 'readable' as const, filePath: directory, isDirectory: true };
    expect(check(owner, 'fileSystem', documentUrl, nativeCheck)).toBe(true);
    expect(check(owner, 'fileSystem', documentUrl, { ...nativeCheck, filePath: '/Users/judekim/Downloads/Other' })).toBe(false);
  });

  it('scopes an origin-only filesystem check to a live owner and clears grants on navigation or destruction', () => {
    const { request, check } = handlers();
    const documentUrl = 'app://powermove/';
    const owner = appContents(documentUrl);
    const directory = '/tmp/powermove-export';
    const selected = { isMainFrame: true, requestingUrl: documentUrl, filePath: directory, isDirectory: true };
    const originOnly = { isMainFrame: false, filePath: `${directory}/frame.png`, isDirectory: false,
      embeddingOrigin: documentUrl };

    expect(check(null, 'fileSystem', documentUrl, originOnly)).toBe(false);
    request(owner, 'fileSystem', vi.fn(), selected);
    expect(check(null, 'fileSystem', documentUrl, originOnly)).toBe(true);
    expect(check(null, 'fileSystem', 'https://example.com', originOnly)).toBe(false);
    expect(check(null, 'fileSystem', documentUrl, { ...originOnly, filePath: '/tmp/other/frame.png' })).toBe(false);

    owner.emit('did-navigate');
    expect(check(null, 'fileSystem', documentUrl, originOnly)).toBe(false);
    request(owner, 'fileSystem', vi.fn(), selected);
    expect(check(null, 'fileSystem', documentUrl, originOnly)).toBe(true);
    owner.emit('destroyed');
    expect(check(null, 'fileSystem', documentUrl, originOnly)).toBe(false);
  });

  it('responds to restricted filesystem access instead of leaving the picker pending', () => {
    const { restricted } = handlers();
    const callback = vi.fn();
    expect(restricted).toBeTypeOf('function');
    (restricted as any)({}, {}, callback);
    expect(callback).toHaveBeenCalledExactlyOnceWith('deny');
  });

  it('denies filesystem requests from subframes and untrusted documents', () => {
    const { request } = handlers();
    const owner = appContents(details.requestingUrl);
    const callback = vi.fn();
    request(owner, 'fileSystem', callback, { ...details, isMainFrame: false, filePath: '/tmp/export', isDirectory: true });
    expect(callback).toHaveBeenLastCalledWith(false);
    request(owner, 'fileSystem', callback, { ...details, requestingUrl: 'https://example.com', filePath: '/tmp/export', isDirectory: true });
    expect(callback).toHaveBeenLastCalledWith(false);
    request(owner, 'fileSystem', callback, { ...details, filePath: 'relative/export', isDirectory: true });
    expect(callback).toHaveBeenLastCalledWith(false);
  });
});
