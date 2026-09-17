<script lang="ts">
  import { inspectorContext } from './context';
  import { ALIGN_LABELS, alignDelta, alignFrame, deltaInParent, type AlignMode, type Box } from './align';

  /* Alignment strip: two rounded segments, horizontal then vertical, with the
     panel showing through the gap between them. One selected layer aligns to
     the composition frame; several align to their common bounds. */

  let { layers }: { layers: any[] } = $props();
  const { api, edit: inspectorEdit } = inspectorContext();

  const HORIZONTAL: AlignMode[] = ['left', 'hcenter', 'right'];
  const VERTICAL: AlignMode[] = ['top', 'vcenter', 'bottom'];

  const GLYPHS: Record<AlignMode, string> = {
    left: 'M3 2.5v11M5.5 5h7M5.5 9h4.5',
    hcenter: 'M8 2.5v11M4.5 5h7M5.75 9h4.5',
    right: 'M13 2.5v11M3.5 5h7M6 9h4.5',
    top: 'M2.5 3h11M5 5.5v7M9 5.5v4.5',
    vcenter: 'M2.5 8h11M5 4.5v7M9 5.75v4.5',
    bottom: 'M2.5 13h11M5 3.5v7M9 6v4.5',
  };

  function worldBox(layer: any, time: number): Box | null {
    if (api.space3d?.is3DLayer?.(layer)) return null;
    const bounds = api.render.gl.bounds(layer, time);
    if (!bounds) return null;
    const m = api.anim.worldMatrix(layer, time);
    const corners = [[bounds.x0, bounds.y0], [bounds.x1, bounds.y0], [bounds.x1, bounds.y1], [bounds.x0, bounds.y1]]
      .map(([x, y]) => ({ x: m[0] * x! + m[2] * y! + m[4], y: m[1] * x! + m[3] * y! + m[5] }));
    return {
      x0: Math.min(...corners.map(p => p.x)), y0: Math.min(...corners.map(p => p.y)),
      x1: Math.max(...corners.map(p => p.x)), y1: Math.max(...corners.map(p => p.y)),
    };
  }

  function align(mode: AlignMode): void {
    const time = api.transport.time();
    const targets = layers.filter(layer => layer && !layer.lock && layer.type !== 'audio');
    const boxed = targets.map(layer => ({ layer, box: worldBox(layer, time) })).filter((item): item is { layer: any; box: Box } => !!item.box);
    const frame = alignFrame(boxed.map(item => item.box), api.project.get());
    if (!frame) return;
    const commands: any[] = [];
    for (const { layer, box } of boxed) {
      const world = alignDelta(box, frame, mode);
      if (!world.dx && !world.dy) continue;
      const parent = layer.parent ? api.model.layer(layer.parent) : null;
      const local = deltaInParent(parent ? api.anim.worldMatrix(parent, time) : null, world.dx, world.dy);
      if (local.dx) commands.push({ type: 'set_property', target: layer.id, path: 'position.x', value: api.util.round(Number(api.anim.ev(layer, 'position.x', time)) + local.dx, 3), time, mode: 'auto', preserveHandEdits: false });
      if (local.dy) commands.push({ type: 'set_property', target: layer.id, path: 'position.y', value: api.util.round(Number(api.anim.ev(layer, 'position.y', time)) + local.dy, 3), time, mode: 'auto', preserveHandEdits: false });
    }
    if (!commands.length) return;
    inspectorEdit.apply(commands, { label: ALIGN_LABELS[mode], origin: 'inspector' });
    api.transport.invalidate?.();
  }
</script>

<div class="align-strip" role="toolbar" aria-label="Align">
  {#each [HORIZONTAL, VERTICAL] as group}
    <div class="align-group">
      {#each group as mode}
        <button type="button" aria-label={ALIGN_LABELS[mode]} title={ALIGN_LABELS[mode]} onclick={() => align(mode)} onpointerdown={(event) => event.stopPropagation()}>
          <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" aria-hidden="true"><path d={GLYPHS[mode]} /></svg>
        </button>
      {/each}
    </div>
  {/each}
</div>

<style>
  /* Each button is its own well; a 1px seam of panel shows between them and
     only the outer corners of a group are rounded. Hover and press are per
     button. A wider gap separates the horizontal and vertical groups. */
  .align-strip { display: flex; gap: 6px; margin: 2px 4px 10px; }
  .align-group { display: flex; flex: 1; gap: 1px; }
  /* Same material as every value well: tinted rest, a touch more on hover,
     pressed a step darker. */
  button { display: flex; align-items: center; justify-content: center; flex: 1; height: 28px; padding: 0; border: 0; border-radius: 0; background: rgb(var(--ink-rgb) / .06); color: var(--tx-3); cursor: default; transition: background var(--dur-1), color var(--dur-1); }
  button:first-child { border-radius: var(--r-sm) 0 0 var(--r-sm); }
  button:last-child { border-radius: 0 var(--r-sm) var(--r-sm) 0; }
  button:hover { background: rgb(var(--ink-rgb) / .085); color: var(--tx); }
  button:active { background: rgb(var(--ink-rgb) / .12); color: var(--tx); transform: scale(.94); transition: none; }
  button:focus-visible { outline: 2px solid var(--accent); outline-offset: -2px; }
</style>
