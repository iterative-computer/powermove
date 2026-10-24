import { expect, it, vi } from 'vitest';
import { buildWaveform } from './media-work';

function buffer(channels: Float32Array[], duration: number) {
  return { length: channels[0]!.length, duration, numberOfChannels: channels.length,
    getChannelData: (channel: number) => channels[channel] };
}

it('preserves peaks across channel and chunk boundaries, including the last sample', async () => {
  const left = new Float32Array(100003), right = new Float32Array(left.length);
  left[0] = .25; right[32768] = -.75; left[left.length - 1] = -1;
  const yieldTask = vi.fn(async () => {}), progress = vi.fn();
  const peaks = await buildWaveform(buffer([left, right], 1), { yieldTask, onProgress: progress });
  expect(peaks.length).toBe(100);
  expect(peaks[0]).toBe(.25); expect(peaks[32]).toBe(.75); expect(peaks[99]).toBe(1);
  expect(yieldTask).toHaveBeenCalledTimes(7);
  expect(progress).toHaveBeenLastCalledWith(200006, 200006);
});

it('lets input tasks run before a large waveform completes', async () => {
  let heartbeat = false;
  setTimeout(() => { heartbeat = true; }, 0);
  const peaks = await buildWaveform(buffer([new Float32Array(2_000_000).fill(.5)], 40));
  expect(heartbeat).toBe(true);
  expect(peaks.every(peak => peak === .5)).toBe(true);
});

it('reads native PCM in bounded blocks instead of materializing entire channels', async () => {
  const pcm = new Float32Array(100003); pcm[32768] = -.75; pcm[100002] = 1;
  const getChannelData = vi.fn(() => { throw new Error('unbounded PCM copy'); });
  const copyFromChannel = vi.fn((target: Float32Array, _channel: number, offset: number) => {
    target.fill(0); target.set(pcm.subarray(offset, offset + target.length));
  });
  const native = { length: pcm.length, duration: 1, numberOfChannels: 1, getChannelData, copyFromChannel };
  const actual = await buildWaveform(native, { yieldTask: async () => {} });
  const expected = await buildWaveform(buffer([pcm], 1), { yieldTask: async () => {} });
  expect(actual).toEqual(expected);
  expect(getChannelData).not.toHaveBeenCalled();
  expect(copyFromChannel.mock.calls.every(([target]) => target.byteLength <= 128 * 1024)).toBe(true);
});

it('checks cancellation inside a huge single bucket and stops without publishing partial peaks', async () => {
  const controller = new AbortController();
  let yields = 0;
  const progress = vi.fn();
  await expect(buildWaveform(buffer([new Float32Array(1_000_000)], .001), {
    assertCurrent: () => controller.signal.throwIfAborted(), onProgress: progress,
    yieldTask: async () => { if (++yields === 2) controller.abort(); },
  })).rejects.toMatchObject({ name: 'AbortError' });
  expect(progress).toHaveBeenCalledExactlyOnceWith(32768, 1_000_000);
});
