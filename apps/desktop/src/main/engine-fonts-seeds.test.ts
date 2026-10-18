import { EventEmitter } from 'node:events';
import { describe, expect, it, vi } from 'vitest';

vi.mock('electron', () => ({}));

import { FontSeedTracker, watchWindowSeeds } from './engine-fonts-seeds';
import { ENGINE_FONTS_EVENTS } from '../shared/engine-fonts-ipc';

class FakeWindow extends EventEmitter {
  destroyed = false;
  constructor(readonly webContents: { id: number }, public bounds: { x: number }) { super(); }
  isDestroyed() { return this.destroyed; }
  getBounds() { return this.bounds; }
}

describe('main\'s window seeds', () => {
  it('track each window\'s displays, tell the window, and send all windows\' seeds to the engines', () => {
    const app = new EventEmitter();
    const screen = Object.assign(new EventEmitter(), {
      // Displays left of x = 1000 are 2x, the rest 1x.
      getDisplayMatching: (b: { x: number }) => ({ scaleFactor: b.x < 1000 ? 2 : 1 })
    });
    const sent: Array<[number, string, string]> = [];
    const contents = (id: number) => ({ isDestroyed: () => false, send: (channel: string, json: string) => void sent.push([id, channel, json]) });
    const engines: string[] = [];
    const first = new FakeWindow({ id: 1 }, { x: 0 });
    const tracker = new FontSeedTracker();
    const stop = watchWindowSeeds(tracker, { app: app as any, screen: screen as any, windows: () => [first as any], contents }, (json) => engines.push(json));
    const sizes = (json: string) => (JSON.parse(json).seeds as Array<{ size: number }>).map(s => s.size);
    expect(sizes(engines.at(-1)!)).toEqual([24, 26]);
    expect(sent).toEqual([[1, ENGINE_FONTS_EVENTS.seedsChanged, engines.at(-1)!]]);
    // A new window on a 1x display.
    const second = new FakeWindow({ id: 2 }, { x: 1200 });
    app.emit('browser-window-created', {}, second);
    expect(sizes(engines.at(-1)!)).toEqual([24, 26, 12, 13]);
    expect(sizes(sent.at(-1)![2])).toEqual([12, 13]);
    // The first window moves to the 1x display: it keeps its 2x entries.
    first.bounds = { x: 1500 };
    first.emit('moved');
    expect(sent.at(-1)![0]).toBe(1);
    expect(sizes(sent.at(-1)![2])).toEqual([24, 26, 12, 13]);
    // Display changes re-check every window; nothing new, nothing sent.
    const count = engines.length;
    screen.emit('display-metrics-changed');
    expect(engines.length).toBe(count);
    // The first window closes: the engines get the second window's seeds.
    first.emit('closed');
    expect(sizes(engines.at(-1)!)).toEqual([12, 13]);
    stop();
    expect(app.listenerCount('browser-window-created')).toBe(0);
    expect(screen.listenerCount('display-metrics-changed')).toBe(0);
  });
});
