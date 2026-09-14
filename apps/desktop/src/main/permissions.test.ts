import { describe, expect, it, vi } from 'vitest';
import type { Session, WebContents } from 'electron';
import { installPermissionHandlers } from './permissions';

function handlers(trusted = true) {
  const session = {
    setPermissionRequestHandler: vi.fn<Session['setPermissionRequestHandler']>(),
    setPermissionCheckHandler: vi.fn<Session['setPermissionCheckHandler']>(),
  };
  const isAppMainFrame = vi.fn(() => trusted);
  installPermissionHandlers(session, isAppMainFrame);
  return {
    request: session.setPermissionRequestHandler.mock.calls[0]![0]!,
    check: session.setPermissionCheckHandler.mock.calls[0]![0]!,
    isAppMainFrame,
  };
}

const contents = {} as WebContents;
const origin = 'app://powermove';
const details = { isMainFrame: true, requestingUrl: `${origin}/index.html` };

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
});
