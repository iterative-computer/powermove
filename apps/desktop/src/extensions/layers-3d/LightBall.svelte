<script lang="ts">
  import type { PowermoveAPI, Scene3DLight } from 'powermove';

  /*
   * The lights as dots on a ball seen from the camera: drag a dot to swing the
   * light around the subject, past the edge to move it behind. The ball is
   * shaded by the lights themselves. Double-click adds a light from that side.
   */
  let { api, lights, focus = null, add }: {
    api: PowermoveAPI;
    lights: Scene3DLight[];
    focus?: string | null;
    add: (direction: [number, number, number]) => void;
  } = $props();

  const SIZE = 132, R = 56, C = SIZE / 2;
  let canvas = $state<HTMLCanvasElement | null>(null);
  let dragging = $state<{ id: string; direction: [number, number, number] } | null>(null);

  const shown = $derived(lights.map((light) => dragging?.id === light.id ? { ...light, direction: dragging.direction } : light));
  const rgb = (hex: string) => {
    const value = /^#([0-9a-f]{6})/i.exec(hex)?.[1] ?? 'ffffff';
    return [0, 2, 4].map((i) => parseInt(value.slice(i, i + 2), 16) / 255) as [number, number, number];
  };

  $effect(() => {
    const context = canvas?.getContext('2d');
    if (!canvas || !context) return;
    const scale = Math.min(2, globalThis.devicePixelRatio || 1), size = Math.round(SIZE * scale), radius = R * scale, center = size / 2;
    canvas.width = size; canvas.height = size;
    const image = context.createImageData(size, size), data = image.data;
    const sources = shown.map((light) => ({ d: light.direction, c: rgb(light.color), i: Math.max(0, light.intensity) }));
    for (let y = 0; y < size; y++) for (let x = 0; x < size; x++) {
      const u = (x + .5 - center) / radius, v = (y + .5 - center) / radius, r2 = u * u + v * v;
      if (r2 > 1) continue;
      // The front of the ball faces the camera (-z in composition axes).
      const n = [u, v, -Math.sqrt(1 - r2)];
      const color = [.06, .06, .065];
      for (const s of sources) {
        const lambert = Math.max(0, n[0]! * s.d[0] + n[1]! * s.d[1] + n[2]! * s.d[2]) * s.i * .42;
        for (let k = 0; k < 3; k++) color[k] = color[k]! + s.c[k]! * lambert;
      }
      const edge = Math.min(1, (1 - Math.sqrt(r2)) * radius), offset = (y * size + x) * 4;
      for (let k = 0; k < 3; k++) data[offset + k] = Math.round(255 * Math.pow(color[k]! / (1 + color[k]!), 1 / 1.6));
      data[offset + 3] = Math.round(255 * edge);
    }
    context.putImageData(image, 0, 0);
  });

  const dot = (direction: [number, number, number]) => ({ x: C + direction[0] * R, y: C + direction[1] * R, behind: direction[2] > 0 });

  /** The ball point under the pointer; past the rim the direction wraps around to the other side. */
  function directionAt(event: PointerEvent, behind: boolean): [number, number, number] {
    const bounds = (event.currentTarget as Element).closest('.light-ball')!.getBoundingClientRect();
    let u = (event.clientX - bounds.left - C) / R, v = (event.clientY - bounds.top - C) / R;
    let r = Math.hypot(u, v), back = behind;
    if (r > 1) {
      const wrapped = 2 - Math.min(2, r);
      u *= wrapped / r; v *= wrapped / r; r = wrapped; back = !behind;
    }
    const z = Math.sqrt(Math.max(0, 1 - r * r)) * (back ? 1 : -1);
    const length = Math.hypot(u, v, z) || 1;
    return [u / length, v / length, z / length];
  }

  function press(event: PointerEvent, light: Scene3DLight) {
    if (event.button !== 0 || light.locked) return;
    event.preventDefault(); event.stopPropagation();
    const target = event.currentTarget as Element, behind = light.direction[2] > 0, label = 'Aim light';
    target.setPointerCapture(event.pointerId);
    try { api.edit.begin(label, { origin: 'inspector' }); } catch (error) { api.ui.toast(String(error), { error: true }); return; }
    dragging = { id: light.id, direction: light.direction };
    const move = (e: PointerEvent) => {
      const direction = directionAt(e, behind);
      dragging = { id: light.id, direction };
      try {
        const result = api.edit.apply(api.scene3d.lighting.aimCommands(light.id, direction), { origin: 'inspector', label });
        if (!result.ok) throw new Error(result.message);
      } catch (error) { api.ui.toast(error instanceof Error ? error.message : String(error), { error: true }); finish(false); }
      api.transport.invalidate();
    };
    const finish = (commit: boolean) => {
      target.removeEventListener('pointermove', move as EventListener);
      target.removeEventListener('pointerup', up as EventListener);
      target.removeEventListener('pointercancel', cancel);
      window.removeEventListener('keydown', key, true);
      if (!dragging) return;
      dragging = null;
      if (commit) api.edit.commit(label); else api.edit.cancel();
      api.transport.invalidate();
    };
    const up = () => finish(true), cancel = () => finish(false);
    const key = (e: KeyboardEvent) => { if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); finish(false); } };
    target.addEventListener('pointermove', move as EventListener);
    target.addEventListener('pointerup', up as EventListener);
    target.addEventListener('pointercancel', cancel);
    window.addEventListener('keydown', key, true);
  }
  function addAt(event: MouseEvent) {
    if ((event.target as Element).closest('.light')) return;
    add(directionAt(event as PointerEvent, false));
  }
</script>

<div class="light-ball" style={`width:${SIZE}px;height:${SIZE}px`} role="group" aria-label="Light directions"
  title="Drag a light to aim it. Double-click to add one." ondblclick={addAt}>
  <canvas bind:this={canvas} aria-hidden="true"></canvas>
  <svg viewBox={`0 0 ${SIZE} ${SIZE}`}>
    {#each shown as light (light.id)}
      {@const p = dot(light.direction)}
      <g class="light" class:behind={p.behind} class:focus={light.id === focus} class:locked={light.locked}
        role="button" tabindex="-1" aria-label={`${light.name}, ${p.behind ? 'behind' : 'in front'}: drag to aim`}
        onpointerdown={(event) => press(event, light)}>
        <circle class="hit" cx={p.x} cy={p.y} r="11" />
        <circle class="mark" cx={p.x} cy={p.y} r={light.id === focus ? 6.5 : 5} fill={light.color} />
      </g>
    {/each}
  </svg>
</div>

<style>
  .light-ball { position: relative; flex: none; margin: 4px 0; touch-action: none; }
  .light-ball canvas, .light-ball svg { position: absolute; inset: 0; width: 100%; height: 100%; }
  .light { cursor: grab; }
  .light:active { cursor: grabbing; }
  .light.locked { cursor: not-allowed; }
  .hit { fill: transparent; }
  .mark { stroke: #fff; stroke-width: 1.5; filter: drop-shadow(0 0 1.5px rgba(0, 0, 0, .7)); }
  .focus .mark { stroke-width: 2.25; }
  .behind .mark { fill-opacity: .25; stroke-dasharray: 2 2; }
  :global(.row.split:has(.light-ball)) { height: auto; align-items: flex-start; }
</style>
