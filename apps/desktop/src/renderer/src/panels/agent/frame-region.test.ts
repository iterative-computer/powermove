import { describe, expect, it } from 'vitest';
import { frameRegionNote, onComposition, regionFromDrag, screenRegionNote } from './frame-region';
import { promptSegments } from './inline-prompt';

const frame = { left: 100, top: 50, width: 960, height: 540 };
const composition = { w: 1920, h: 1080 };

describe('regionFromDrag', () => {
  it('converts a viewer drag into composition pixels', () => {
    expect(regionFromDrag({ x: 580, y: 320 }, { x: 340, y: 140 }, frame, composition))
      .toEqual({ x: 480, y: 180, width: 480, height: 360 });
  });

  it('clamps a drag that leaves the frame', () => {
    expect(regionFromDrag({ x: 0, y: 0 }, { x: 580, y: 320 }, frame, composition))
      .toEqual({ x: 0, y: 0, width: 960, height: 540 });
  });

  it('treats a click or an off-frame drag as the whole frame', () => {
    const whole = { x: 0, y: 0, width: 1920, height: 1080 };
    expect(regionFromDrag({ x: 300, y: 300 }, { x: 302, y: 303 }, frame, composition)).toEqual(whole);
    expect(regionFromDrag({ x: 0, y: 0 }, { x: 80, y: 40 }, frame, composition)).toEqual(whole);
  });
});

describe('frameRegionNote', () => {
  it('names the time, frame and area', () => {
    const note = frameRegionNote({ x: 480, y: 180, width: 480, height: 360 }, { ...composition, fps: 30, name: 'Main' }, 3.4, '00:03:12');
    expect(note).toBe('composition "Main" at 00:03:12 (3.400s, frame 102 at 30 fps); selected area x 480–960, y 180–540 px of 1920×1080, outlined in orange');
  });
});

describe('onComposition', () => {
  it('needs most of the box inside the frame', () => {
    expect(onComposition({ x: 150, y: 100, width: 200, height: 100 }, { x: 150, y: 100 }, frame)).toBe(true);
    expect(onComposition({ x: 0, y: 0, width: 200, height: 200 }, { x: 0, y: 0 }, frame)).toBe(false);
    expect(onComposition(null, { x: 500, y: 300 }, frame)).toBe(true);
    expect(onComposition(null, { x: 20, y: 300 }, frame)).toBe(false);
    expect(onComposition(null, { x: 500, y: 300 }, null)).toBe(false);
  });
});

describe('notes', () => {
  it('lists the layers under a composition selection', () => {
    const note = frameRegionNote({ x: 0, y: 0, width: 1920, height: 1080 }, { ...composition, fps: 30 }, 1, '00:01:00', ['"Sun" (solid, id a)']);
    expect(note).toBe('whole frame of composition at 00:01:00 (1.000s, frame 30 at 30 fps), 1920×1080; layers there: "Sun" (solid, id a)');
  });

  it('describes a window selection by area and panels', () => {
    expect(screenRegionNote({ x: 10, y: 20, width: 300, height: 200 }, { width: 1440, height: 900 }, ['Timeline', 'Viewer']))
      .toBe('Powermove window area x 10–310, y 20–220 of 1440×900 CSS px; covers panels: Timeline, Viewer');
  });
});

describe('noted attachment marks', () => {
  it('still place the attachment in the transcript', () => {
    const item = { name: 'Frame 00:03:12 selection.jpg' };
    const { segments, loose } = promptSegments(`look [Attachment: ${item.name} | composition at 00:03:12] here`, [item]);
    expect(loose).toEqual([]);
    expect(segments).toEqual([{ text: 'look ' }, { attachment: item }, { text: ' here' }]);
  });
});
