import { describe, expect, it, vi } from 'vitest';
import { EventEmitter } from 'node:events';

vi.mock('electron', () => ({
  BrowserWindow: { fromWebContents: vi.fn() },
  dialog: {}
}));

import { openProjectForWindow } from './save';
import { FILE_CHUNK_BYTES } from './file-upload';

function sender() {
  let destroyed = false;
  const owner = Object.assign(new EventEmitter(), {
    isDestroyed: () => destroyed,
    destroy() {
      destroyed = true;
      owner.emit('destroyed');
    }
  });
  return owner;
}

describe('external project open', () => {
  it.each([FILE_CHUNK_BYTES, FILE_CHUNK_BYTES * 2])('omits the whole document from a %i-byte PMV4 bridge response', async length => {
    const document = { proj: {}, history: { note: 'synthetic'.repeat(length / 8) } };
    const documentRange = { offset: 5, length, sha256: 'a'.repeat(64) };
    const opened = { path: '/tmp/large.pmv', projectId: 'project-1', token: 'large-reader', size: length + 100,
      document, media: [], documentRange };
    const projects = { open: vi.fn(async () => opened), close: vi.fn(async () => undefined) };
    const owner = sender();
    const result = await openProjectForWindow({ projects: projects as any }, owner as any, opened.path);
    expect(Object.hasOwn(result, 'document')).toBe(false);
    expect(result).toEqual({ ok: true, path: opened.path, projectId: opened.projectId, token: opened.token,
      size: opened.size, media: [], documentRange });
    expect(opened.document).toBe(document);
    expect(projects.open).toHaveBeenCalledWith(opened.path, { allowDocumentRange: true });
    owner.destroy();
  });

  it('passes a worker-validated range-only result through the bridge without inventing a document', async () => {
    const opened = { path: '/tmp/worker.pmv', projectId: 'worker-project', token: 'worker-reader', size: 64 * 1024 * 1024 + 100,
      media: [], documentRange: { offset: 5, length: 64 * 1024 * 1024, sha256: 'a'.repeat(64) } };
    const projects = { open: vi.fn(async () => opened), close: vi.fn(async () => undefined) }, owner = sender();
    const result = await openProjectForWindow({ projects: projects as any }, owner as any, opened.path);
    expect(result).toEqual({ ok: true, ...opened });
    expect(result).not.toHaveProperty('document');
    owner.destroy();
    await vi.waitFor(() => expect(projects.close).toHaveBeenCalledWith(opened.token, false));
  });

  it.each(['small PMV4', 'PMV3 or legacy'])('preserves the %s document in the bridge response', async format => {
    const document = { proj: {}, history: { entries: [] } };
    const opened = { path: '/tmp/small.pmv', projectId: 'project-1', token: 'small-reader', size: FILE_CHUNK_BYTES + 100,
      document, media: [], ...(format === 'small PMV4'
        ? { documentRange: { offset: 5, length: FILE_CHUNK_BYTES - 1, sha256: 'a'.repeat(64) } } : {}) };
    const projects = { open: vi.fn(async () => opened), close: vi.fn(async () => undefined) };
    const owner = sender();
    const result = await openProjectForWindow({ projects: projects as any }, owner as any, opened.path);
    expect(result).toEqual({ ok: true, ...opened });
    expect(result).toHaveProperty('document', document);
    owner.destroy();
  });

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
