<script lang="ts">
  import { EditGesture, type EditBinding } from '../../controls/gesture';
  import type { EasingCurve, GeneratedCurveControl } from '../../core/types/workspace';
  import { doc } from '../../state/document.svelte';
  import { transport } from '../../state/transport.svelte';

  let {
    PM,
    control,
    get,
    edit
  }: {
    PM: Record<string, any>;
    control: GeneratedCurveControl;
    get: () => unknown;
    edit: EditBinding;
  } = $props();

  const minY = $derived(Number.isFinite(control.minY) ? control.minY : -1);
  const maxY = $derived(Number.isFinite(control.maxY) ? control.maxY : 2);
  const pad = { x: 34, y: 24 };
  const defaultCurve: EasingCurve = [.62, .05, 0, 1];
  const gesture = $derived(new EditGesture(PM, edit));
  let canvas = $state<HTMLCanvasElement>();
  let curve = $state<EasingCurve>([...defaultCurve]);
  let active = $state(0);
  let dragging = false;

  const sanitize = (value: unknown, fallback: EasingCurve = curve): EasingCurve =>
    (PM.Capabilities?.sanitizeCurve?.(value, fallback) || fallback || defaultCurve) as EasingCurve;
  const round = (value: number, places: number): number =>
    typeof PM.round === 'function' ? PM.round(value, places) : Number(value.toFixed(places));
  const clamp = (value: number, min: number, max: number): number =>
    typeof PM.clamp === 'function' ? PM.clamp(value, min, max) : Math.max(min, Math.min(max, value));
  const x = (value: number): number => pad.x + value * ((canvas?.width ?? 600) - pad.x * 2);
  const y = (value: number): number => (canvas?.height ?? 300) - pad.y
    - (value - minY) / (maxY - minY) * ((canvas?.height ?? 300) - pad.y * 2);
  const curveName = $derived(PM.Ease?.nameOf?.([curve[0], curve[1]], [curve[2], curve[3]]) || 'custom');
  const curveText = $derived(curveName === 'custom' ? curve.map((value) => round(value, 2)).join('  ') : curveName);
  const activeText = $derived(`Handle ${active + 1}: x ${curve[active * 2]}, y ${curve[active * 2 + 1]}`);

  function sameCurve(left: EasingCurve, right: EasingCurve): boolean {
    return left.every((value, index) => Object.is(value, right[index]));
  }

  function draw(): void {
    if (!canvas) return;
    const context = canvas.getContext('2d');
    if (!context) return;
    const style = window.getComputedStyle(canvas);
    const ink = style.getPropertyValue('--tx').trim() || '#222';
    const sub = style.getPropertyValue('--tx-3').trim() || '#888';
    const line = style.getPropertyValue('--line').trim() || 'rgba(127,127,127,.2)';
    const accent = style.getPropertyValue('--accent').trim() || '#ff6b1a';
    context.clearRect(0, 0, canvas.width, canvas.height);
    context.lineWidth = 1;
    context.strokeStyle = line;
    for (let column = 0; column <= 4; column++) {
      const gridX = x(column / 4);
      context.beginPath();
      context.moveTo(gridX, pad.y);
      context.lineTo(gridX, canvas.height - pad.y);
      context.stroke();
    }
    for (const value of [minY, 0, .5, 1, maxY]) {
      if (value < minY || value > maxY) continue;
      const gridY = y(value);
      context.beginPath();
      context.moveTo(pad.x, gridY);
      context.lineTo(canvas.width - pad.x, gridY);
      context.stroke();
    }
    const points: [[number, number], [number, number]] = [
      [x(curve[0]), y(curve[1])],
      [x(curve[2]), y(curve[3])]
    ];
    context.strokeStyle = sub;
    context.lineWidth = 2;
    context.beginPath();
    context.moveTo(x(0), y(0));
    context.lineTo(...points[0]);
    context.moveTo(x(1), y(1));
    context.lineTo(...points[1]);
    context.stroke();
    const easing = PM.Ease?.bezier?.(...curve) || ((value: number) => value);
    context.strokeStyle = accent;
    context.lineWidth = 4;
    context.beginPath();
    for (let index = 0; index <= 120; index++) {
      const time = index / 120;
      if (index) context.lineTo(x(time), y(easing(time)));
      else context.moveTo(x(time), y(easing(time)));
    }
    context.stroke();
    points.forEach((handle, index) => {
      context.fillStyle = index === active ? accent : ink;
      context.beginPath();
      context.arc(handle[0], handle[1], index === active ? 9 : 7, 0, Math.PI * 2);
      context.fill();
      context.strokeStyle = style.getPropertyValue('--bg-panel').trim() || '#fff';
      context.lineWidth = 3;
      context.stroke();
    });
  }

  function point(event: PointerEvent): { x: number; y: number } {
    const rect = canvas!.getBoundingClientRect();
    return {
      x: (event.clientX - rect.left) * canvas!.width / Math.max(1, rect.width),
      y: (event.clientY - rect.top) * canvas!.height / Math.max(1, rect.height)
    };
  }

  function update(next: unknown, mode: 'write' | 'once' | 'sync' = 'once'): void {
    const clean = sanitize(next);
    curve = [...clean] as EasingCurve;
    if (mode === 'write') gesture.write([...clean]);
    else if (mode === 'once') gesture.once([...clean]);
    draw();
    if (mode !== 'write') PM.invalidate?.('render');
  }

  function moveHandle(event: PointerEvent): void {
    const position = point(event);
    const next = [...curve] as EasingCurve;
    next[active * 2] = round(clamp((position.x - pad.x) / (canvas!.width - pad.x * 2), 0, 1), 3);
    next[active * 2 + 1] = round(clamp(
      minY + (canvas!.height - pad.y - position.y) / (canvas!.height - pad.y * 2) * (maxY - minY),
      minY,
      maxY
    ), 3);
    update(next, 'write');
  }

  function pointerdown(event: PointerEvent): void {
    if (event.button !== 0 || !canvas) return;
    const position = point(event);
    const first: [number, number] = [x(curve[0]), y(curve[1])];
    const second: [number, number] = [x(curve[2]), y(curve[3])];
    active = Math.hypot(position.x - second[0], position.y - second[1])
      < Math.hypot(position.x - first[0], position.y - first[1]) ? 1 : 0;
    dragging = true;
    gesture.begin();
    canvas.setPointerCapture?.(event.pointerId);
    canvas.focus();
    moveHandle(event);
    event.preventDefault();
  }

  function release(event: PointerEvent, cancel = false): void {
    if (!dragging) return;
    dragging = false;
    canvas?.releasePointerCapture?.(event.pointerId);
    if (cancel) gesture.cancel();
    else gesture.commit();
  }

  function keydown(event: KeyboardEvent): void {
    if (event.key === 'Enter' || event.key === ' ') {
      active = active ? 0 : 1;
      draw();
      event.preventDefault();
      return;
    }
    const axis = ['ArrowLeft', 'ArrowRight'].includes(event.key) ? 0
      : ['ArrowUp', 'ArrowDown'].includes(event.key) ? 1 : -1;
    if (axis < 0) return;
    const next = [...curve] as EasingCurve;
    const direction = ['ArrowLeft', 'ArrowDown'].includes(event.key) ? -1 : 1;
    const valueIndex = active * 2 + axis;
    next[valueIndex] = (next[valueIndex] ?? 0) + direction * (event.shiftKey ? .1 : .01);
    update(next);
    event.preventDefault();
  }

  $effect(() => {
    doc.tick.values;
    doc.proj;
    transport.time;
    const next = sanitize(get(), curve) || curve || defaultCurve;
    if (!dragging && !sameCurve(curve, next)) curve = [...next] as EasingCurve;
    draw();
  });
</script>

<section class="generated-curve-field">
  <div class="generated-curve-head"><span>{control.label}</span><code>{curveText}</code></div>
  <canvas
    bind:this={canvas}
    class="generated-curve-canvas"
    width="600"
    height="300"
    tabindex="0"
    role="slider"
    aria-label={`${control.label}. ${activeText}. Press Enter to switch handles.`}
    aria-valuemin={minY}
    aria-valuemax={maxY}
    aria-valuenow={curve[active * 2 + 1]}
    aria-valuetext={`${curveName}: ${curve.join(', ')}`}
    onpointerdown={pointerdown}
    onpointermove={(event) => { if (dragging) moveHandle(event); }}
    onpointerup={(event) => release(event)}
    onpointercancel={(event) => release(event, true)}
    onkeydown={keydown}
  ></canvas>
  <div class="generated-curve-presets" role="group" aria-label={`${control.label} presets`}>
    {#each control.presets as name (name)}
      <button type="button" title={`Use ${name} easing`} onclick={() => update(PM.Ease.PRESETS[name])}>
        <i aria-hidden="true"></i><span>{name}</span>
      </button>
    {/each}
  </div>
</section>

<style>
  .generated-curve-canvas:focus-visible {
    outline-width: 2px;
  }
</style>
