/*
 * Phase 4 bridge: RuntimePorts implemented over the Svelte stores, fed by the
 * LEGACY app while it still owns PM.proj / PM.sel / PM.time.
 *
 * Direction is strictly legacy → runes in this phase. Svelte panels never write
 * runes directly; they call the same PM.Edit / PM.hist / transport functions
 * the legacy chrome calls, and the bus adapter below reflects the result.
 * (Phase 5 flips the direction, then deletes the bus.)
 */
import type { Project } from '../core/types/project';
import { doc } from '../state/document.svelte';
import { sel, setSelection } from '../state/selection.svelte';
import { perf, transport } from '../state/transport.svelte';
import { frameBus } from './frame-bus';
import { frame, invalidate } from './invalidate';
import { installPorts, type MutationKind, type RuntimePorts, type Selection } from './ports';

type LegacyPM = Record<string, any>;

const BUS_TO_TICK: Record<string, MutationKind> = {
  layers: 'structure',
  project: 'project',
  assets: 'assets',
  library: 'library',
  history: 'history',
  'draw:ui': 'values'
};

function currentSelection(PM: LegacyPM): Selection {
  return {
    layers: [...(PM.sel?.layers ?? [])],
    keys: [...(PM.sel?.keys ?? [])].filter((k) => typeof k === 'string'),
    chan: PM.sel?.chan ?? null
  };
}

/** Mirror the legacy project reference into the raw store when it changes. */
function syncProjectRef(PM: LegacyPM): void {
  if (PM.proj && PM.proj !== doc.proj) doc.replace(PM.proj as Project);
}

export function installLegacyRuntime(PM: LegacyPM): () => void {
  const ports: RuntimePorts = {
    getProject: () => PM.proj as Project,
    replaceProject: (next) => {
      if (typeof PM.replaceProject === 'function') PM.replaceProject(next);
      else PM.proj = next;
      syncProjectRef(PM);
    },
    getSelection: () => currentSelection(PM),
    setSelection: (next) => {
      PM.selectLayers(next.layers);
      PM.sel.keys = [...next.keys];
      PM.sel.chan = next.chan;
      setSelection(next);
    },
    getTime: () => PM.time as number,
    setTime: (t) => {
      PM.setTime(t);
    },
    invalidate,
    frame
  };
  installPorts(ports);

  syncProjectRef(PM);
  setSelection(currentSelection(PM));
  transport.time = PM.time ?? 0;
  transport.playing = !!PM.playing;
  transport.quality = PM.quality ?? 1;
  transport.tool = PM.tool ?? 'select';

  const offs: Array<() => void> = [];
  const on = (event: string, fn: (...args: any[]) => void): void => {
    offs.push(PM.bus.on(event, fn));
  };

  for (const [event, kind] of Object.entries(BUS_TO_TICK)) {
    on(event, () => {
      syncProjectRef(PM);
      doc.bumpFor(kind);
    });
  }
  on('sel', () => setSelection(currentSelection(PM)));
  on('time', (t: number) => {
    transport.time = t;
  });
  on('transport', () => {
    transport.playing = !!PM.playing;
  });
  on('quality', () => {
    transport.quality = PM.quality ?? 1;
    frameBus.emit('quality');
  });
  on('tool', () => {
    transport.tool = PM.tool ?? 'select';
  });
  on('draw:status', () => {
    const stats = PM.GL?.stats ?? {};
    perf.ms = PM.perf?.ms ?? stats.ms ?? 0;
    perf.fps = PM.perf?.fps ?? 0;
    perf.draws = stats.draws ?? 0;
    perf.passes = stats.passes ?? 0;
    perf.progs = stats.progs ?? 0;
    perf.raster = typeof PM.rasterStats === 'function' ? (PM.rasterStats().size ?? 0) : 0;
    frameBus.emit('status');
  });
  on('draw:timeline', () => frameBus.emit('timeline'));
  on('overlay', () => frameBus.emit('overlay'));
  on('layout', () => frameBus.emit('layout'));
  on('layout:applied', () => frameBus.emit('layout-applied'));

  return () => {
    for (const off of offs) off();
  };
}
