import { expect, it, vi } from 'vitest';
import { cancelPreviewVideoSeek, previewSeekFrame, seekPreviewVideo } from './video-seek';

it('finishes the current decode then seeks only the latest requested frame', () => {
  const video = new EventTarget() as any;
  let time = 0;
  const seeks: number[] = [];
  Object.assign(video, { paused: true, seeking: false });
  Object.defineProperty(video, 'currentTime', {
    get: () => time,
    set: value => { time = value; seeks.push(value); video.seeking = true; },
  });
  seekPreviewVideo(video, 1, .0005);
  seekPreviewVideo(video, 2, .0005);
  seekPreviewVideo(video, 3, .0005);
  expect(seeks).toEqual([1]);
  video.seeking = false; video.dispatchEvent(new Event('seeked'));
  expect(seeks).toEqual([1, 3]);
  video.seeking = false; video.dispatchEvent(new Event('seeked'));
  expect(seeks).toEqual([1, 3]);
  seekPreviewVideo(video, 3 + 1 / 120, .0005);
  expect(seeks.at(-1)).toBeCloseTo(3 + 1 / 120);
});

it('does not apply an old paused scrub request after playback starts', () => {
  const video = new EventTarget() as any;
  Object.assign(video, { paused: true, seeking: true, currentTime: 0 });
  seekPreviewVideo(video, 1, .0005);
  video.paused = false; video.seeking = false; video.currentTime = 2;
  video.dispatchEvent(new Event('seeked'));
  expect(video.currentTime).toBe(2);
});

it('lets offline frame preparation own a paused decoder', () => {
  const video = new EventTarget() as any;
  Object.assign(video, { paused: true, seeking: true, currentTime: 0 });
  seekPreviewVideo(video, 1, .0005);
  cancelPreviewVideoSeek(video);
  video.seeking = false; video.currentTime = 4;
  video.dispatchEvent(new Event('seeked'));
  expect(video.currentTime).toBe(4);
});

it('retries a trim seek once a cold decoder has metadata', () => {
  const video = new EventTarget() as any;
  let time = 0, loaded = false;
  Object.assign(video, { paused: true, seeking: false });
  Object.defineProperty(video, 'currentTime', {
    get: () => time,
    set: value => { if (loaded) time = value; },
  });
  seekPreviewVideo(video, 1, .0005);
  expect(time).toBe(0);
  loaded = true;
  video.dispatchEvent(new Event('loadedmetadata'));
  expect(time).toBe(1);
});

it('keeps completed pixels while a slow decoder starts the newest queued seek', () => {
  const video = new EventTarget() as any;
  let time = 0;
  Object.assign(video, { paused: true, seeking: false, readyState: 2, videoWidth: 64, videoHeight: 64 });
  Object.defineProperty(video, 'currentTime', {
    get: () => time,
    set: value => { time = value; video.seeking = true; video.readyState = 1; },
  });
  const decoded: number[] = [];
  const canvas = { width: 0, height: 0, getContext: () => ({ drawImage: () => decoded.push(video.currentTime) }) };
  vi.stubGlobal('document', { createElement: () => canvas });
  try {
    seekPreviewVideo(video, 1, .0005);
    seekPreviewVideo(video, 2, .0005);
    video.seeking = false; video.readyState = 2; video.dispatchEvent(new Event('seeked'));
    expect(decoded).toEqual([1]);
    expect(video.currentTime).toBe(2);
    expect(video.readyState).toBe(1);
    expect(previewSeekFrame(video)).toMatchObject({ canvas, time: 1, version: 1 });
    video.seeking = false; video.readyState = 2; video.dispatchEvent(new Event('seeked'));
    expect(previewSeekFrame(video)).toMatchObject({ time: 2, version: 2 });
    cancelPreviewVideoSeek(video);
    expect(previewSeekFrame(video)).toBeUndefined();
    expect(canvas.width).toBe(0);
  } finally { vi.unstubAllGlobals(); }
});

it('uses only a matching decoded frame during rapid forward and reverse scrubs', () => {
  const video = new EventTarget() as any;
  let time = 0;
  const seeks: number[] = [];
  Object.assign(video, { paused: true, seeking: false, readyState: 2, videoWidth: 64, videoHeight: 64 });
  Object.defineProperty(video, 'currentTime', {
    get: () => time,
    set: value => { time = value; seeks.push(value); video.seeking = true; video.readyState = 1; },
  });
  vi.stubGlobal('document', { createElement: () => ({ width: 0, height: 0, getContext: () => ({ drawImage() {} }) }) });
  try {
    seekPreviewVideo(video, 1, .0005);
    seekPreviewVideo(video, 2, .0005);
    video.seeking = false; video.readyState = 2; video.dispatchEvent(new Event('seeked'));
    expect(previewSeekFrame(video, 1)?.time).toBe(1);
    expect(previewSeekFrame(video, 2)).toBeUndefined();
    expect(seeks).toEqual([1, 2]);

    // The first frame remains available even though the second decode is in flight.
    seekPreviewVideo(video, 1, .0005);
    expect(previewSeekFrame(video, 1)?.time).toBe(1);
    video.seeking = false; video.readyState = 2; video.dispatchEvent(new Event('seeked'));
    expect(seeks).toEqual([1, 2]);
    expect(previewSeekFrame(video, 1)?.time).toBe(1);
    expect(previewSeekFrame(video, 2)?.time).toBe(2);
    expect(previewSeekFrame(video, 3)).toBeUndefined();
  } finally { vi.unstubAllGlobals(); }
});
