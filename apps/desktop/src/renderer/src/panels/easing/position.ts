import type { EditCommand } from '../../core/types/commands';
import type { Keyframe } from '../../core/types/project';
import { linearKeyframeEase } from '../../core/anim/temporal-ease';
import { createPathTiming } from '../../core/anim/motion-path';
import { clone } from './curve';
import { command, tracks, type Track } from './operations';

type PM = Record<string, any>;
interface Path { layerId: string; from: number; start: number; end: number; axes: Track[] }
function paths(PM: PM): Path[] {
  const selection = tracks(PM).filter(t => /^position\.[xyz]$/.test(t.path));
  if (!selection.length) throw new Error('Select two or more consecutive Position keyframes');
  const result: Path[] = [];
  for (const layerId of new Set(selection.map(t => t.layerId))) {
    const selected = selection.filter(t => t.layerId === layerId);
    const keys = selected.flatMap(t => t.keys.filter(k => t.selected.includes(k.i)));
    const start = Math.min(...keys.map(k => k.t)), end = Math.max(...keys.map(k => k.t));
    if (!(end > start)) throw new Error('Select a position movement with at least two different times');
    const layer = PM.L(layerId);
    const axes = ['x', 'y', 'z'].flatMap(axis => {
      const path = `position.${axis}`, prop = layer.p[path];
      if (!prop) return [];
      if (prop.expr && !prop.expr.includes(`motionPath(${JSON.stringify(layerId)},${JSON.stringify(path)},`)) throw new Error('Remove existing position expressions before creating a path controller');
      return [{ layerId, from: layer.from, path, keys: prop.kf, selected: prop.kf.filter((k: Keyframe) => k.t >= start && k.t <= end).map((k: Keyframe) => k.i) }];
    });
    result.push({ layerId, from: layer.from, start, end, axes });
  }
  return result;
}
/** Only remove knots when the route can be proven to be one straight,
 * monotonic line. Curved geometry and all its native handles stay intact. */
function cleaned(PM: PM, path: Path): EditCommand[] {
  const layer = PM.L(path.layerId);
  const sample = (t: number): number[] => path.axes.map(axis => {
    const prop = layer.p[axis.path]; return prop.kf.length ? Number(PM.evalKfs(prop.kf, t)) : Number(prop.v);
  });
  const times = [...new Set([path.start, ...path.axes.flatMap(axis => axis.keys.filter(k => k.t > path.start && k.t < path.end).map(k => k.t)), path.end])].sort((a, b) => a - b);
  // Validate timing before generating commands, so failure is atomic.
  createPathTiming(sample, times, path.start, path.end);
  const first = sample(path.start), last = sample(path.end), direction = last.map((v, i) => v - first[i]!);
  const span = Math.hypot(...direction);
  if (!(span > 0) || !Number.isFinite(span)) return [];
  for (const axis of path.axes) {
    if (!axis.keys.length) continue;
    if (!axis.keys.some(k => k.t === path.start) || !axis.keys.some(k => k.t === path.end)) return [];
    for (let i = 0; i < axis.keys.length - 1; i++) {
      const a = axis.keys[i]!, b = axis.keys[i + 1]!;
      if (b.t <= path.start || a.t >= path.end) continue;
      if (a.outInterp !== 'linear' || b.inInterp !== 'linear' || (a as any).spring) return [];
    }
  }
  let previous = -Infinity;
  for (const time of times) {
    const position = sample(time), u = position.reduce((n, v, i) => n + ((v - first[i]!) / span) * (direction[i]! / span), 0);
    const tolerance = Number.EPSILON * 128 * Math.max(1, ...position.map(Math.abs), ...first.map(Math.abs), ...last.map(Math.abs));
    if (u < previous || u < 0 || u > 1 || position.some((v, i) => Math.abs(v - first[i]! - u * direction[i]!) > tolerance)) return [];
    previous = u;
  }
  return path.axes.filter(axis => axis.keys.length).map(axis => {
    const start = axis.keys.find(k => k.t === path.start)!, end = axis.keys.find(k => k.t === path.end)!;
    const a: Keyframe = { ...clone(start), outInterp: 'linear', autoBezier: false, continuous: false };
    const b: Keyframe = { ...clone(end), inInterp: 'linear', autoBezier: false, continuous: false };
    return command(axis, axis.keys.filter(k => k.t < path.start || k.t > path.end).concat([a, b]).sort((a, b) => a.t - b.t));
  });
}
const reference = (path: Path, axis: Track, progress: string) =>
  `motionPath(${JSON.stringify(path.layerId)},${JSON.stringify(axis.path)},${progress},${path.start},${path.end})`;
function controllerCommands(PM: PM, path: Path, cleanup: boolean): EditCommand[] {
  const layer = PM.L(path.layerId);
  const existing = PM.curComp().layers.find((candidate: any) => candidate.type === 'null'
    && candidate.d?.easingPath?.layerId === layer.id && candidate.d.easingPath.start === path.start && candidate.d.easingPath.end === path.end
    && path.axes.some(axis => layer.p[axis.path].expr?.includes(`sample(${JSON.stringify(candidate.id)},"opacity",t)`)));
  const id = existing?.id || PM.uid('l');
  const keys: Keyframe[] = [
    { ...linearKeyframeEase(), i: existing?.p.opacity.kf[0]?.i || PM.uid('k'), t: path.start, v: 0 },
    { ...linearKeyframeEase(), i: existing?.p.opacity.kf.at(-1)?.i || PM.uid('k'), t: path.end, v: 100 }
  ];
  const commands: EditCommand[] = [
    ...(existing ? [] : [{ type: 'add_layer' as const, id, layerType: 'null' as const, name: `${layer.name} · Path progress`, from: layer.from, duration: layer.dur, select: false,
      content: { easingPath: { layerId: layer.id, start: path.start, end: path.end } } }]),
    ...(!existing || cleanup ? [command({ layerId: id, from: layer.from, path: 'opacity', keys, selected: [] })] : []),
    ...cleaned(PM, path),
    ...path.axes.map(axis => ({ type: 'set_expression' as const, target: axis.layerId, path: axis.path,
      expression: `${cleanup ? `t<${path.start}||t>${path.end}?value:` : ''}${reference(path, axis, `clamp(sample(${JSON.stringify(id)},"opacity",t)/100,0,1)`)}` }))
  ];
  return commands;
}
export function cleanupPath(PM: PM): EditCommand[] { return paths(PM).flatMap(path => controllerCommands(PM, path, true)); }
export function positionController(PM: PM): EditCommand[] { return paths(PM).flatMap(path => controllerCommands(PM, path, false)); }
