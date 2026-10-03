<script lang="ts">
  // A few tall upright cases frame the step like buildings. `edge` is the distance from the window edge to the case centre;
  // cases further back sink by their depth so their bases stay below the window.
  type Case = { side: 'left' | 'right'; edge: number; z: number; w: number; h: number; d: number; delay: number; trolley?: boolean; tag?: boolean };
  type Face = { kind: 'front' | 'arc' | 'side'; x: number; z: number; ry: number; width: number; fill: string };

  const cases: Case[] = [
    { side: 'left', edge: 70, z: -300, w: 90, h: 880, d: 80, delay: 140 },
    { side: 'left', edge: 30, z: 0, w: 170, h: 560, d: 80, delay: 0, tag: true },
    { side: 'right', edge: 70, z: -300, w: 90, h: 780, d: 80, delay: 220, trolley: true },
    { side: 'right', edge: 35, z: 0, w: 160, h: 510, d: 80, delay: 80 }
  ];

  // The inner vertical edge is a rounded corner built from thin strips. Every surface is lit by a point light sitting just in
  // front of the content in the centre of the window, so the faces turned toward the content catch the most light.
  const RADIUS = 16, STEPS = 8;
  const LIGHT = { x: 450, z: 140 }, EYE = { x: 450, z: 1000 };
  const unit = (x: number, z: number): [number, number] => { const l = Math.hypot(x, z); return [x / l, z / l]; };
  const rad = (deg: number) => deg * Math.PI / 180;

  // Shade a point on a case (case-local x/z) with outward normal (nx, nz). Positions are measured from each case's own window
  // edge, so the right-hand cases are mirrored into the same frame.
  function shade(c: Case, px: number, pz: number, nx: number, nz: number, gloss: boolean) {
    const left = c.side === 'left';
    const x = left ? c.edge - c.w / 2 + px : c.edge + c.w / 2 - px, z = c.z + pz, mx = left ? nx : -nx;
    const dist = Math.hypot(LIGHT.x - x, LIGHT.z - z), light = unit(LIGHT.x - x, LIGHT.z - z), eye = unit(EYE.x - x, EYE.z - z);
    const diffuse = Math.max(0, mx * light[0] + nz * light[1]) / (1 + (dist / 600) ** 2);
    const half = unit(light[0] + eye[0], light[1] + eye[1]);
    const spec = gloss ? Math.pow(Math.max(0, mx * half[0] + nz * half[1]), 48) * 40 : 0;
    let fill = `color-mix(in oklab, var(--c) ${(58 + 42 * Math.min(1, diffuse * 1.5)).toFixed(1)}%, black)`;
    if (spec > 0.5) fill = `color-mix(in oklab, ${fill}, white ${spec.toFixed(1)}%)`;
    // Cases further back pick up a little haze.
    return c.z < 0 ? `color-mix(in oklab, ${fill}, var(--bg-window) 14%)` : fill;
  }

  // Each face is shaded at both ends so light falls off smoothly across it.
  const ramp = (from: string, to: string) => `linear-gradient(90deg, ${from}, ${to})`;

  function faces(c: Case): Face[] {
    const left = c.side === 'left', step = 90 / STEPS, chord = 2 * RADIUS * Math.sin(rad(step / 2));
    const cx = left ? c.w - RADIUS : RADIUS, cz = c.d / 2 - RADIUS, start = left ? 0 : -90;
    const fx = left ? 0 : RADIUS, fw = c.w - RADIUS;
    const out: Face[] = [{ kind: 'front', x: fx, z: c.d / 2, ry: 0, width: fw, fill: ramp(shade(c, fx, c.d / 2, 0, 1, false), shade(c, fx + fw, c.d / 2, 0, 1, false)) }];
    for (let i = 0; i < STEPS; i++) {
      const a = rad(start + i * step), b = rad(start + (i + 1) * step);
      const ax = cx + RADIUS * Math.sin(a), az = cz + RADIUS * Math.cos(a);
      const fill = ramp(shade(c, ax, az, Math.sin(a), Math.cos(a), true), shade(c, cx + RADIUS * Math.sin(b), cz + RADIUS * Math.cos(b), Math.sin(b), Math.cos(b), true));
      out.push({ kind: 'arc', x: ax, z: az, ry: start + (i + 0.5) * step, width: chord + 0.6, fill });
    }
    out.push(left
      ? { kind: 'side', x: c.w, z: cz, ry: 90, width: c.d - RADIUS, fill: ramp(shade(c, c.w, cz, 1, 0, false), shade(c, c.w, -c.d / 2, 1, 0, false)) }
      : { kind: 'side', x: 0, z: -c.d / 2, ry: -90, width: c.d - RADIUS, fill: ramp(shade(c, 0, -c.d / 2, -1, 0, false), shade(c, 0, cz, -1, 0, false)) });
    return out;
  }
</script>

<div class="scene" aria-hidden="true">
  {#each cases as item}
    <div
      class="case {item.side}" class:far={item.z < 0}
      style:--edge="{item.edge}px" style:--z="{item.z}px" style:--w="{item.w}px" style:--h="{item.h}px" style:--d="{item.d}px"
      style:--delay="{item.delay}ms"
    >
      {#each faces(item) as face}
        <span class="face {face.kind}" style:width="{face.width}px" style:--fill={face.fill} style:transform="translate3d({face.x}px, 0, {face.z}px) rotateY({face.ry}deg)">
          {#if face.kind === 'front' && item.tag}<i class="tag"></i>{/if}
          {#if face.kind === 'side'}<i class="grip"></i>{/if}
        </span>
      {/each}
      {#if item.trolley}<i class="trolley"></i>{/if}
    </div>
  {/each}
</div>

<style>
  .scene { position: fixed; inset: 0; z-index: 0; overflow: hidden; pointer-events: none; perspective: 1000px; perspective-origin: 50% 45%; --c: color-mix(in oklab, var(--tx) 30%, var(--bg-window)); }
  /* Cases slide outward as the window narrows so the centre stays clear. */
  .case { --shift: clamp(0px, (900px - 100vw) * .45, 90px); position: absolute; bottom: calc(var(--z) - 40px); width: var(--w); height: var(--h); transform-style: preserve-3d; transform: translateZ(var(--z)); animation: rise 1100ms cubic-bezier(.16, 1, .3, 1) var(--delay) backwards; }
  .case.left { left: calc(var(--edge) - var(--w) / 2 - var(--shift)); }
  .case.right { right: calc(var(--edge) - var(--w) / 2 - var(--shift)); }
  /* The light sits level with the content, so faces dim gently above it and more toward the floor. */
  .face { --falloff: linear-gradient(rgb(0 0 0 / .08), transparent 28%, transparent 45%, rgb(0 0 0 / .2)); position: absolute; top: 0; left: 0; display: block; height: var(--h); transform-origin: 0 0; background: var(--falloff), var(--fill); }
  .front {
    --ribs: repeating-linear-gradient(90deg, transparent 0 calc(25% - 4px), rgb(0 0 0 / .05) calc(25% - 4px), rgb(0 0 0 / .16) calc(25% - 1px), rgb(255 255 255 / .2) calc(25% - 1px) 25%);
    background: var(--falloff), var(--ribs), var(--fill);
    background-position: 0 0, 12.5% 0, 0 0;
  }
  /* Ambient occlusion where a back case stands close beside the near one. */
  .left.far .front { background: var(--falloff), linear-gradient(90deg, rgb(0 0 0 / .16), transparent 45%), var(--ribs), var(--fill); background-position: 0 0, 0 0, 12.5% 0, 0 0; }
  .right.far .front { background: var(--falloff), linear-gradient(270deg, rgb(0 0 0 / .16), transparent 45%), var(--ribs), var(--fill); background-position: 0 0, 0 0, 12.5% 0, 0 0; }
  /* A moulded seam runs down the middle of each side. */
  .side { --seam: linear-gradient(90deg, transparent calc(50% - 1.5px), rgb(0 0 0 / .22) calc(50% - 1.5px) calc(50% + .5px), rgb(255 255 255 / .12) calc(50% + .5px) calc(50% + 1.5px), transparent calc(50% + 1.5px)); }
  .side { background: var(--falloff), var(--seam), var(--fill); }
  .grip { position: absolute; left: 50%; top: 18%; width: 12px; height: 70px; transform: translateX(-50%); border-radius: 6px; background: linear-gradient(90deg, color-mix(in oklab, var(--c), black 55%), color-mix(in oklab, var(--c), black 32%) 45%, color-mix(in oklab, var(--c), black 50%)); box-shadow: 0 4px 6px rgb(0 0 0 / .22), inset 0 1px 0 rgb(255 255 255 / .12); }
  .trolley { position: absolute; left: 30%; right: 30%; bottom: 100%; height: 90px; transform: translateZ(calc(var(--d) / -4)); border: 6px solid color-mix(in oklab, var(--c), black 38%); border-bottom: 0; border-radius: 8px 8px 0 0; }
  .tag { position: absolute; right: 16px; top: 40px; width: 26px; height: 42px; border-radius: 4px 4px 6px 6px; background: linear-gradient(160deg, color-mix(in oklab, var(--c), white 55%), color-mix(in oklab, var(--c), white 35%)); box-shadow: 0 6px 10px rgb(0 0 0 / .2), 0 1px 2px rgb(0 0 0 / .18); transform: rotate(8deg); transform-origin: 50% 0; }
  .tag::before { content: ''; position: absolute; left: 50%; top: 6px; width: 6px; height: 6px; margin-left: -3px; border-radius: 50%; background: color-mix(in oklab, var(--c), black 20%); box-shadow: inset 0 1px 1px rgb(0 0 0 / .3); }
  @keyframes rise { from { translate: 0 110vh; } }
  @media (prefers-reduced-motion: reduce) { .case { animation: none; } }
</style>
