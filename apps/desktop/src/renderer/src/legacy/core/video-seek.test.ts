import { expect, it } from 'vitest';
import { cancelPreviewVideoSeek, seekPreviewVideo } from './video-seek';

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
