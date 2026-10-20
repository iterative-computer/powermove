<script lang="ts">
  import { bounds, clamp, clone, cubic, insertPoint, path, type Curve } from './curve';
  import { handleControlPoint, setHandleFromPoint } from '../../core/anim/temporal-ease';
  let { curve, onChange }: { curve: Curve; onChange: (next: Curve, phase: 'begin' | 'update' | 'end' | 'cancel') => void } = $props();
  let svg: SVGSVGElement;
  let active = $state(0);
  let side = $state<'in' | 'out' | null>('out');
  let dragging = $state(false);
  let initial: Curve | undefined;
  let range: [number, number] = [0, 1];
  const extent = $derived(dragging ? range : bounds(curve));
  const low = $derived(extent[0]), high = $derived(extent[1]);
  const x = (t: number) => 9 + t * 278;
  const y = (v: number) => 169 - (v - low) / (high - low) * 160;
  const handles = $derived(curve.points.flatMap((p, i) => (['in', 'out'] as const).flatMap(s => {
    const h = handleControlPoint(curve.points, i, s);
    return h ? [{ i, side: s, t: h.t, v: h.v, key: p }] : [];
  })));
  function position(event: PointerEvent) {
    const r = svg.getBoundingClientRect();
    return { t: clamp(((event.clientX - r.left) / r.width * 296 - 9) / 278, 0, 1),
      v: clamp(low + (169 - (event.clientY - r.top) / r.height * 178) / 160 * (high - low), -10, 10) };
  }
  function begin(event: PointerEvent, index?: number, which?: 'in' | 'out') {
    if (event.button !== 0) return;
    event.preventDefault(); event.stopPropagation();
    svg.focus();
    const p = position(event);
    if (event.altKey && !event.metaKey && !which) {
      const next = index === undefined ? insertPoint(curve, p.t) : clone(curve);
      if (index !== undefined && index > 0 && index < next.points.length - 1) next.points.splice(index, 1);
      onChange(next, 'begin'); onChange(next, 'update'); onChange(next, 'end'); return;
    }
    if (index === undefined) return;
    initial = clone(curve); range = bounds(curve); active = index; side = which || null;
    if (event.metaKey && !which) side = index === curve.points.length - 1 ? 'in' : 'out';
    dragging = true; svg.setPointerCapture(event.pointerId); onChange(curve, 'begin');
    if (event.metaKey && !which) move(event);
  }
  function move(event: PointerEvent) {
    if (!dragging || !initial) return;
    const p = position(event), next = clone(curve), key = next.points[active]!;
    if (side) {
      if (event.shiftKey) p.v = key.v;
      setHandleFromPoint(next.points, active, side, p, { breakContinuity: event.metaKey });
      if (event.metaKey && event.altKey && next.points.length === 2) {
        setHandleFromPoint(next.points, active ? 0 : 1, active ? 'out' : 'in', { t: 1 - p.t, v: 1 - p.v });
      }
    } else if (active > 0 && active < next.points.length - 1) {
      key.t = clamp(p.t, next.points[active - 1]!.t + .005, next.points[active + 1]!.t - .005);
      key.v = event.shiftKey ? initial.points[active]!.v : p.v;
    }
    onChange(next, 'update');
  }
  function finish(event: PointerEvent, cancel = false) {
    if (!dragging) return;
    dragging = false;
    if (svg.hasPointerCapture(event.pointerId)) svg.releasePointerCapture(event.pointerId);
    onChange(cancel ? initial! : curve, cancel ? 'cancel' : 'end'); initial = undefined;
  }
  function keydown(event: KeyboardEvent) {
    if (event.key === 'Tab') return;
    event.stopPropagation();
    if (event.key === 'Escape' && dragging) { dragging = false; onChange(initial!, 'cancel'); initial = undefined; return; }
    if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); active = (active + 1) % curve.points.length; side = active === curve.points.length - 1 ? 'in' : 'out'; return; }
    if ((event.key === 'Delete' || event.key === 'Backspace') && active > 0 && active < curve.points.length - 1) {
      event.preventDefault(); const next = clone(curve); next.points.splice(active, 1); active = 0;
      onChange(curve, 'begin'); onChange(next, 'update'); onChange(next, 'end'); return;
    }
    if (!event.key.startsWith('Arrow')) return;
    event.preventDefault();
    const next = clone(curve), key = next.points[active]!, delta = event.shiftKey ? .1 : .01;
    const h = side ? handleControlPoint(next.points, active, side) : key;
    if (!h) return;
    if (event.key === 'ArrowLeft') h.t -= delta;
    if (event.key === 'ArrowRight') h.t += delta;
    if (event.key === 'ArrowUp') h.v += delta;
    if (event.key === 'ArrowDown') h.v -= delta;
    if (side) setHandleFromPoint(next.points, active, side, h);
    else if (active > 0 && active < next.points.length - 1) { key.t = clamp(h.t, next.points[active - 1]!.t + .005, next.points[active + 1]!.t - .005); key.v = h.v; }
    onChange(curve, 'begin'); onChange(next, 'update'); onChange(next, 'end');
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions (The graph is a keyboard-operable application with documented arrow and Enter controls.) -->
<svg bind:this={svg} class="easing-graph" viewBox="0 0 296 178" role="application" tabindex="0"
  aria-label="Easing graph. Drag handles. Option-click to add or remove points. Enter cycles points; arrows adjust the active handle."
  onpointerdown={e => begin(e)} onpointermove={move} onpointerup={e => finish(e)} onpointercancel={e => finish(e, true)} onkeydown={keydown}
  ondblclick={() => { const next = cubic(); onChange(curve, 'begin'); onChange(next, 'update'); onChange(next, 'end'); }}>
  <title>Easing curve editor</title>
  <g class="grid">
    {#each [0, .25, .5, .75, 1] as t}<path d={`M${x(t)} 9V169`} />{/each}
    {#each [0, .25, .5, .75, 1] as v}<path d={`M9 ${y(low + v * (high - low))}H287`} />{/each}
  </g>
  {#each handles as h}
    <path class="handle-line" d={`M${x(h.key.t)} ${y(h.key.v)}L${x(h.t)} ${y(h.v)}`} />
  {/each}
  <path class="curve" d={path(curve, 296, 178, 9, low, high)} />
  {#each handles as h}
    <circle class="hit" cx={x(h.t)} cy={y(h.v)} r="8" onpointerdown={e => begin(e, h.i, h.side)} role="presentation" />
    <circle class="handle" class:active={h.i === active && h.side === side} cx={x(h.t)} cy={y(h.v)} r="3.5" />
  {/each}
  {#each curve.points as p, i}
    <rect class="hit" x={x(p.t) - 7} y={y(p.v) - 7} width="14" height="14" onpointerdown={e => begin(e, i)} role="presentation" />
    <rect class="point" x={x(p.t) - 2.5} y={y(p.v) - 2.5} width="5" height="5" />
  {/each}
</svg>

<style>
  .easing-graph { display:block; width:100%; height:auto; aspect-ratio:296 / 178; background:var(--bg-field); border-radius:4px; touch-action:none; outline:none; }
  .easing-graph:focus-visible { outline:2px solid var(--accent); outline-offset:2px; }
  .grid { fill:none; stroke:var(--line-2); stroke-width:.75; }
  .curve { fill:none; stroke:var(--tx); stroke-width:1.8; pointer-events:none; }
  .handle-line { fill:none; stroke:var(--accent); stroke-dasharray:3 3; stroke-width:1; pointer-events:none; }
  .handle { fill:var(--accent); pointer-events:none; }
  .point { fill:var(--tx); pointer-events:none; }
  .hit { fill:transparent; cursor:grab; }
  .hit:active { cursor:grabbing; }
</style>
