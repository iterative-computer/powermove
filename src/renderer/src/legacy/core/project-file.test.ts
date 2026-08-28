import { describe, expect, it, vi } from 'vitest';
import { restoreProjectFileMedia } from './project-file';

describe('portable project media', () => {
  it('restores a video-sized payload without overflowing the regex stack', async () => {
    const put = vi.fn(async (_id: string, _blob: Blob, _metadata: unknown) => true);
    const data = 'AAAA'.repeat(5_000_000);
    await restoreProjectFileMedia({
      proj: { assets: { video: { id: 'video', name: 'video.mp4' } } },
      media: { video: { type: 'video/mp4', data } },
    }, { get: async () => null, put });
    expect(put).toHaveBeenCalledOnce();
    expect(put.mock.calls[0]?.[1].size).toBe(15_000_000);
    expect(put.mock.calls[0]?.[1].type).toBe('video/mp4');
  });

  it.each(['a', 'AA=A', 'AAAA====', '!!!!', 'AAA\n', 'AA==AAAA'])('rejects malformed media before writing: %s', async data => {
    const put = vi.fn(async () => true);
    await expect(restoreProjectFileMedia({
      proj: { assets: { image: { id: 'image' } } },
      media: { image: { type: 'image/png', data } },
    }, { get: async () => null, put })).rejects.toThrow('Invalid saved media');
    expect(put).not.toHaveBeenCalled();
  });

  it('keeps legacy project files compatible', async () => {
    const put = vi.fn(async () => true);
    await restoreProjectFileMedia({ proj: { assets: {} } }, { get: async () => null, put });
    expect(put).not.toHaveBeenCalled();
  });
});
