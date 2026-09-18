import { describe, expect, it, vi } from 'vitest';

vi.mock('electron', () => ({
  BrowserWindow: { fromWebContents: vi.fn() },
  dialog: {}
}));

import { openProjectForWindow } from './save';

function sender() {
  let destroyed = false;
  const handlers = new Map<string, () => void>();
  return {
    isDestroyed: () => destroyed,
    once: vi.fn((event: string, callback: () => void) => { handlers.set(event, callback); }),
    destroy() {
      destroyed = true;
      handlers.get('destroyed')?.();
    }
  };
}

describe('external project open', () => {
  it('returns the opened path and reader token', async () => {
    const projects = {
      open: vi.fn(async (filePath: string) => ({
        path: filePath, projectId: 'project-1', token: 'reader-1', size: 42,
        document: { proj: {} }, media: []
      })),
      close: vi.fn()
    };
    const result = await openProjectForWindow({ projects: projects as any }, sender() as any, '/tmp/example.pmv');

    expect(result).toMatchObject({ ok: true, path: '/tmp/example.pmv', token: 'reader-1' });
  });

  it('returns a structured failure when opening fails', async () => {
    const projects = { open: vi.fn(async () => { throw new Error('Unreadable project'); }), close: vi.fn() };
    const result = await openProjectForWindow({ projects: projects as any }, sender() as any, '/tmp/broken.pmv');

    expect(result).toEqual({ ok: false, cancelled: false, error: 'Unreadable project' });
  });

  it('closes the reader when its sender is destroyed', async () => {
    const projects = {
      open: vi.fn(async (filePath: string) => ({
        path: filePath, projectId: 'project-1', token: 'reader-2', size: 42,
        document: { proj: {} }, media: []
      })),
      close: vi.fn(async () => undefined)
    };
    const owner = sender();
    await openProjectForWindow({ projects: projects as any }, owner as any, '/tmp/example.pmv');
    owner.destroy();
    await vi.waitFor(() => expect(projects.close).toHaveBeenCalledWith('reader-2', false));
  });
});
