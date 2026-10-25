// @vitest-environment happy-dom
/*
 * The engine's audio health (issue A5) in the status bar and the
 * performance panel: from `RenderStats.audio` through `EngineStatsView`, the
 * runtime's `draw:status` mirror, to the two existing surfaces.
 */
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, describe, expect, it } from 'vitest';

import type { AudioDiagnostics, RenderStats } from '../../../shared/engine-protocol';
import PerfPanel from '../panels/PerfPanel.svelte';
import { frameBus } from '../runtime/frame-bus';
import { installLegacyRuntime } from '../runtime/install-legacy';
import { resetPortsForTests } from '../runtime/ports';
import StatusBar from '../shell/StatusBar.svelte';
import { audioStatus, updateAudioHealth } from '../state/transport.svelte';
import { audioHealth, audioPerfRows, audioStatusField, sameAudioHealth } from './audio-diagnostics';
import { EngineStatsView } from './presentation-events';

const DROP = 'Audio decode exceeded 120 ms; missing voice plays as silence: music';

function diagnostics(over: Partial<AudioDiagnostics> = {}): AudioDiagnostics {
  return { problems: [], starvedRenders: 0, underruns: 0, deviceOverloads: 0, ...over } as AudioDiagnostics;
}

function stats(frames: number, audio?: AudioDiagnostics): RenderStats {
  return {
    frames, timedFrames: 0, lateSkipped: 0, cpuUs: 1_000, gpuUs: 1_000, layers: 2, frameUsAvg: 2_000,
    programs: 1, rasters: 0, layerFailures: 0, snapshots: 0, snapshotUsMax: 0, revision: 1,
    ...(audio ? { audio } : {})
  } as unknown as RenderStats;
}

afterEach(() => {
  updateAudioHealth(null);
  frameBus.clear();
  resetPortsForTests(null);
  document.body.replaceChildren();
});

describe('audio health', () => {
  it('is absent without engine audio, and healthy playback shows nothing in the status bar', () => {
    expect(audioHealth(undefined)).toBeNull();
    expect(audioStatusField(null)).toBeNull();
    expect(audioPerfRows(null)).toEqual([]);
    const healthy = audioHealth(diagnostics());
    expect(audioStatusField(healthy)).toBeNull();
    expect(audioPerfRows(healthy).map((r) => [r.label, r.value])).toEqual([['Underruns', 0], ['Overloads', 0], ['Audio waits', 0]]);
  });

  it('a held segment that recovered (starved renders alone) stays out of the status bar', () => {
    const health = audioHealth(diagnostics({ starvedRenders: 5 }));
    expect(audioStatusField(health)).toBeNull();
    expect(audioPerfRows(health).find((r) => r.label === 'Audio waits')?.value).toBe(5);
  });

  it('a dropped voice (A3) is a problem, listed in the tooltip and as its own panel row', () => {
    const health = audioHealth(diagnostics({ problems: [DROP], starvedRenders: 1, underruns: 2, deviceOverloads: 1 }));
    expect(audioStatusField(health)).toEqual({
      text: 'Audio · 1 problem',
      title: `${DROP}\n2 buffer underruns and 1 device overload since audio started.`
    });
    const rows = audioPerfRows(health);
    expect(rows.at(-1)).toEqual({ label: 'Audio issue', value: DROP, title: DROP });
    const two = audioHealth(diagnostics({ problems: [DROP, 'mix: decoder failed'] }));
    expect(audioStatusField(two)?.text).toBe('Audio · 2 problems');
  });

  it('dropouts alone are counted, singular and plural', () => {
    expect(audioStatusField(audioHealth(diagnostics({ underruns: 1 })))).toEqual({
      text: 'Audio · 1 dropout', title: '1 buffer underrun since audio started.'
    });
    expect(audioStatusField(audioHealth(diagnostics({ underruns: 2, deviceOverloads: 3 })))?.text).toBe('Audio · 5 dropouts');
  });

  it('a device that did not open comes first', () => {
    const error = 'no audio output: no default output device';
    const health = audioHealth(diagnostics({ error, problems: [DROP] }));
    expect(audioStatusField(health)).toEqual({ text: 'No audio output', title: `${error}\n${DROP}` });
    expect(audioPerfRows(health)[0]).toEqual({ label: 'Audio output', value: 'Unavailable', title: error });
  });

  it('compares by value, so an unchanged report replaces nothing', () => {
    const a = audioHealth(diagnostics({ problems: [DROP], underruns: 1 }));
    expect(sameAudioHealth(a, audioHealth(diagnostics({ problems: [DROP], underruns: 1 })))).toBe(true);
    expect(sameAudioHealth(a, audioHealth(diagnostics({ problems: [DROP], underruns: 2 })))).toBe(false);
    expect(sameAudioHealth(a, null)).toBe(false);
    expect(sameAudioHealth(null, null)).toBe(true);
  });
});

describe('EngineStatsView audio', () => {
  it('follows the latest stats message, including one whose session has no audio', () => {
    const view = new EngineStatsView();
    expect(view.audio()).toBeNull();
    view.receive(stats(10, diagnostics({ problems: [DROP], starvedRenders: 3 })), 1_000);
    expect(view.audio()).toEqual({ error: '', problems: [DROP], starvedRenders: 3, underruns: 0, deviceOverloads: 0 });
    // A paused session reports audio changes with no new frame.
    view.receive(stats(10, diagnostics({ problems: [DROP], starvedRenders: 3, underruns: 1 })), 1_600);
    expect(view.audio()?.underruns).toBe(1);
    view.receive(stats(11), 2_200);
    expect(view.audio()).toBeNull();
  });
});

function fakePM() {
  const listeners = new Map<string, Set<(...a: any[]) => void>>();
  return {
    bus: {
      on(ev: string, fn: (...a: any[]) => void) {
        let s = listeners.get(ev);
        if (!s) listeners.set(ev, (s = new Set()));
        s.add(fn);
        return () => s!.delete(fn);
      },
      emit(ev: string, ...a: any[]) {
        for (const fn of [...(listeners.get(ev) ?? [])]) fn(...a);
      }
    },
    proj: { id: 'p1', w: 1920, h: 1080, fps: 30, layers: [], dur: 10 },
    sel: { layers: [], keys: [], chan: null },
    time: 0,
    playing: false,
    app: { dirty: false },
    GL: { gl: {}, stats: { ms: 1, draws: 1, passes: 0, progs: 0 } },
    round: (value: number) => value,
    invalidate: () => {},
    EngineStats: null as EngineStatsView | null
  } as Record<string, any>;
}

describe('the status bar and the performance panel', () => {
  it('show engine audio problems on the status tick, and clear when the engine stops reporting', () => {
    const PM = fakePM();
    const off = installLegacyRuntime(PM);
    const bar = document.createElement('div');
    const panel = document.createElement('div');
    document.body.append(bar, panel);
    const barInstance = mount(StatusBar, { target: bar, props: { PM } });
    const panelInstance = mount(PerfPanel, { target: panel, props: { panelId: 'perf', spec: {} as any } });
    const fields = () => [...bar.querySelectorAll('.status-field')].map((node) => node.textContent?.trim());
    const rows = () => [...panel.querySelectorAll('.row.split')].map((row) => [
      row.querySelector('.k')?.textContent,
      row.querySelector('.vwrap')?.textContent
    ]);
    try {
      // Today's engine: no audio health anywhere.
      flushSync(() => PM.bus.emit('draw:status'));
      expect(audioStatus.health).toBeNull();
      expect(fields().some((f) => f?.startsWith('Audio'))).toBe(false);
      expect(rows().some(([k]) => k === 'Underruns')).toBe(false);

      // The engine reports a dropped voice while paused.
      const engine = new EngineStatsView();
      PM.EngineStats = engine;
      engine.receive(stats(4, diagnostics({ problems: [DROP], starvedRenders: 2, underruns: 1 })), 1_000);
      flushSync(() => PM.bus.emit('draw:status'));
      expect(fields()).toContain('Audio · 1 problem');
      const field = [...bar.querySelectorAll('.status-field')].find((f) => f.textContent === 'Audio · 1 problem')!;
      expect(field.tagName).toBe('SPAN');
      expect(field.getAttribute('title')).toBe(`${DROP}\n1 buffer underrun since audio started.`);
      expect(rows()).toEqual(expect.arrayContaining([
        ['Underruns', '1'], ['Overloads', '0'], ['Audio waits', '2'], ['Audio issue', DROP]
      ]));
      const issue = [...panel.querySelectorAll('.row.split')].find((row) => row.querySelector('.k')?.textContent === 'Audio issue')!;
      expect(issue.getAttribute('title')).toBe(DROP);
      // The counts the panel always had stay first.
      expect(rows().slice(0, 2).map(([k]) => k)).toEqual(['FPS', 'Frame ms']);

      // The new engine turned off: nothing stays behind.
      PM.EngineStats = null;
      flushSync(() => PM.bus.emit('draw:status'));
      expect(fields().some((f) => f?.startsWith('Audio'))).toBe(false);
      expect(rows().some(([k]) => k === 'Audio issue')).toBe(false);
    } finally {
      unmount(barInstance);
      unmount(panelInstance);
      off();
    }
  });
});
