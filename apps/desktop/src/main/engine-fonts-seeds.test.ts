import { EventEmitter } from 'node:events';
import { describe, expect, it, vi } from 'vitest';

vi.mock('electron', () => ({}));

import { CensusIntake, FontSeedTracker, watchWindowSeeds } from './engine-fonts-seeds';
import { ENGINE_FONTS_EVENTS } from '../shared/engine-fonts-ipc';

class FakeWindow extends EventEmitter {
  destroyed = false;
  readonly webContents: EventEmitter & { id: number };
  constructor(id: number) { super(); this.webContents = Object.assign(new EventEmitter(), { id }); }
  isDestroyed() { return this.destroyed; }
}

const UI = ['sf pro text', '-apple-system', 'blinkmacsystemfont', 'helvetica neue', 'sans-serif'];
const census = (dpr: number, ...sizes: Array<[number, number]>) => JSON.stringify({ dpr, uses: sizes.map(([size, weight]) => ({ families: UI, size, weight, italic: false })) });

describe('main\'s window seeds', () => {
  it('follow each window\'s census, tell the window, and send all windows\' seeds to the engines', async () => {
    const app = new EventEmitter();
    const sent: Array<[number, string, string]> = [];
    const contents = (id: number) => ({ isDestroyed: () => false, send: (channel: string, json: string) => void sent.push([id, channel, json]) });
    const engines: string[] = [];
    const first = new FakeWindow(1);
    const tracker = new FontSeedTracker();
    let generation = 1;
    let installed = ['Helvetica Neue', '.SF NS'];
    const familyNames = vi.fn(async () => installed);
    const intake = new CensusIntake(tracker, { familyNames, generation: async () => generation }, () => undefined);
    const stop = watchWindowSeeds(tracker, { app: app as any, windows: () => [first as any], contents }, (json) => engines.push(json));
    const sizes = (json: string) => (JSON.parse(json).seeds as Array<{ size: number }>).map(s => s.size);
    const owners = (json: string) => Object.fromEntries((JSON.parse(json).owners as Array<{ owner: number; seeds: Array<{ size: number }> }>).map(o => [o.owner, o.seeds.map(s => s.size)]));
    expect(sizes(engines.at(-1)!)).toEqual([]);
    // A comfy window on a 2x display reports its UI text.
    expect(await intake.report(1, census(2, [12, 400], [14, 400], [15, 500]))).toBe(true);
    expect(sizes(engines.at(-1)!)).toEqual([24, 28, 30]);
    expect(owners(engines.at(-1)!)).toEqual({ 1: [24, 28, 30] });
    expect(sent.at(-1)).toEqual([1, ENGINE_FONTS_EVENTS.seedsChanged, JSON.stringify({ seeds: JSON.parse(engines.at(-1)!).seeds })]);
    // A second window on a 1x display.
    const second = new FakeWindow(2);
    app.emit('browser-window-created', {}, second);
    await intake.report(2, census(1, [13, 400]));
    expect(owners(engines.at(-1)!)).toEqual({ 1: [24, 28, 30], 2: [13] });
    expect(familyNames).toHaveBeenCalledTimes(1);
    // SF Pro Text is installed: the UI draws in it, not the system font.
    generation = 2;
    installed = [...installed, 'SF Pro Text'];
    await intake.report(2, census(1, [11, 400]));
    expect(familyNames).toHaveBeenCalledTimes(2);
    expect(owners(engines.at(-1)!)).toEqual({ 1: [], 2: [] });
    installed = installed.filter((n) => n !== 'SF Pro Text');
    generation = 3;
    await intake.report(1, census(2, [12, 400]));
    expect(owners(engines.at(-1)!)).toEqual({ 1: [24, 28, 30], 2: [13, 11] });
    // A malformed message is refused.
    expect(await intake.report(1, '{')).toBe(false);
    // The first window's renderer process goes away: a new one starts empty.
    first.webContents.emit('render-process-gone');
    expect(owners(engines.at(-1)!)).toEqual({ 2: [13, 11] });
    // The second window closes.
    second.emit('closed');
    expect(sizes(engines.at(-1)!)).toEqual([]);
    stop();
    expect(app.listenerCount('browser-window-created')).toBe(0);
  });
});
