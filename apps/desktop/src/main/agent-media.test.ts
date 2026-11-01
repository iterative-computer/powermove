import { mkdtemp, rm, symlink, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { agentFileUrl } from '../shared/agent-media';
import { agentMediaFile, serveAgentMedia } from './agent-media';

let dir = '';
beforeAll(async () => {
  dir = await mkdtemp(path.join(os.tmpdir(), 'agent-media-'));
  await writeFile(path.join(dir, 'shot.png'), Buffer.from('0123456789'));
  await writeFile(path.join(dir, 'notes.txt'), 'secret');
  await symlink(path.join(dir, 'notes.txt'), path.join(dir, 'link.png'));
});
afterAll(() => rm(dir, { recursive: true, force: true }));

const get = (file: string, init?: RequestInit) => {
  const url = agentFileUrl(file);
  return serveAgentMedia(new URL(url), new Request(url, init));
};

describe('agent media', () => {
  it('serves a local image as inert bytes', async () => {
    const response = await get(path.join(dir, 'shot.png'));
    expect(response.status).toBe(200);
    expect(response.headers.get('Content-Type')).toBe('image/png');
    expect(response.headers.get('X-Content-Type-Options')).toBe('nosniff');
    expect(response.headers.get('Content-Security-Policy')).toContain('sandbox');
    expect(await response.text()).toBe('0123456789');
  });

  it('answers byte ranges so video can seek', async () => {
    const open = await get(path.join(dir, 'shot.png'), { headers: { Range: 'bytes=4-' } });
    expect(open.status).toBe(206);
    expect(open.headers.get('Content-Range')).toBe('bytes 4-9/10');
    expect(await open.text()).toBe('456789');
    expect((await get(path.join(dir, 'shot.png'), { headers: { Range: 'bytes=20-' } })).status).toBe(416);
  });

  it('refuses non-media files, relative paths, traversal, symlinks and missing files', async () => {
    expect((await get(path.join(dir, 'notes.txt'))).status).toBe(404);
    expect((await get(path.join(dir, 'link.png'))).status).toBe(404);
    expect((await get(path.join(dir, 'gone.png'))).status).toBe(404);
    expect(agentMediaFile(new URL(agentFileUrl('shots/a.png')))).toBeNull();
    expect(agentMediaFile(new URL(agentFileUrl(`${dir}/../x.png`)))).toBeNull();
    expect(agentMediaFile(new URL(agentFileUrl('~/a.png')))).toBe(path.join(os.homedir(), 'a.png'));
  });
});
