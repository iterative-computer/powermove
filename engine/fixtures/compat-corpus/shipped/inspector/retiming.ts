import { isProperty } from 'powermove';
import type { Layer, PowermoveAPI, TimelineService } from 'powermove';
import type { InspectorRuntimeService } from './context';

const integrals = new WeakMap<object, { version: number; step: number; sums: number[] }>();

function sourceTime(api: PowermoveAPI, layer: Layer, time: number): number {
  const content = api.anim.resolveContent(layer, time) as Record<string, any>;
  const durable = layer.d as Record<string, any>;
  const local = time - layer.from;
  if (content.timeRemap && isProperty(durable.sourceTime)) return Number(api.anim.evP(layer, durable.sourceTime, time, 'c.sourceTime')) || 0;
  const speed = durable.speed;
  if (!isProperty(speed) || (!speed.kf.length && !speed.expr)) return local * Number(content.speed ?? 1) + Number(content.trim || 0);
  const end = Math.max(0, local);
  const step = 1 / (Math.max(30, api.project.get().fps) * 2);
  const version = api.anim.version();
  let cached = integrals.get(speed);
  if (!cached || cached.version !== version || cached.step !== step) {
    cached = { version, step, sums: [0] };
    integrals.set(speed, cached);
  }
  const full = Math.floor(end / step);
  for (let index = cached.sums.length - 1; index < full; index++) {
    cached.sums.push(cached.sums[index]! + Number(api.anim.evP(layer, speed, layer.from + (index + .5) * step, 'c.speed')) * step);
  }
  const remainder = end - full * step;
  return cached.sums[full]! + Number(api.anim.evP(layer, speed, layer.from + full * step + remainder / 2, 'c.speed')) * remainder + Number(content.trim || 0);
}

export function enableTimeRemap(api: PowermoveAPI, layer: Layer, mode: 'remap' | 'freeze' | 'reverse' = 'remap'): void {
  const project = api.project.get();
  const start = sourceTime(api, layer, layer.from);
  const end = sourceTime(api, layer, layer.from + Math.max(0, layer.dur - 1 / project.fps));
  const current = sourceTime(api, layer, api.transport.time());
  const frame = 1 / project.fps;
  const content = layer.d as Record<string, any>;
  api.edit.mutate(mode === 'freeze' ? 'Freeze frame' : mode === 'reverse' ? 'Reverse time' : 'Enable time remapping', () => {
    content.timeRemap = api.model.P(true);
    content.sourceTime = api.model.P(mode === 'freeze' ? current : start);
    if (mode !== 'freeze') {
      api.anim.setKeyOn(content.sourceTime, 0, mode === 'reverse' ? end : start, 'linear', project.fps);
      api.anim.setKeyOn(content.sourceTime, Math.max(frame, layer.dur - frame), mode === 'reverse' ? start : end, 'linear', project.fps);
    } else {
      const key = api.anim.setKeyOn(content.sourceTime, api.transport.time() - layer.from, current, 'linear', project.fps);
      if (key) (key as typeof key & { hold?: boolean }).hold = true;
    }
    api.anim.touch();
  }, { origin: 'inspector' });
  api.services.get<TimelineService>('timeline')?.reveal(layer, ['c.sourceTime']);
  api.services.get<InspectorRuntimeService>('inspector')?.refresh();
  api.transport.invalidate();
}
