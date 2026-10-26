<script lang="ts">
  import { inspectorContext } from './context';
  import { ALIGN_LABELS, alignDelta, deltaInParent, unionBox, type AlignMode, type Box } from './align';
  import { isInspectorLayerEditable } from './multi-edit';

  /* Alignment strip: two rounded segments, horizontal then vertical, with the
     panel showing through the gap between them. One selected layer aligns to
     the composition frame; several move together so their common bounding box
     meets it, keeping their relative positions. */

  let { layers }: { layers: any[] } = $props();
  const { api, edit: inspectorEdit } = inspectorContext();

  const HORIZONTAL: AlignMode[] = ['left', 'hcenter', 'right'];
  const VERTICAL: AlignMode[] = ['top', 'vcenter', 'bottom'];

  /* Phosphor's align glyphs, from the app's icon registry. */
  const GLYPHS: Record<AlignMode, string> = {
    left: 'alignLeft', hcenter: 'alignCenterH', right: 'alignRight',
    top: 'alignTop', vcenter: 'alignCenterV', bottom: 'alignBottom',
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
    const targets = layers.filter(layer => layer && isInspectorLayerEditable(api, layer) && layer.type !== 'audio');
    const boxed = targets.map(layer => ({ layer, box: worldBox(layer, time) })).filter((item): item is { layer: any; box: Box } => !!item.box);
    const bounds = unionBox(boxed.map(item => item.box));
    if (!bounds) return;
    const { w, h } = api.project.get();
    const world = alignDelta(bounds, { x0: 0, y0: 0, x1: w, y1: h }, mode);
    if (!world.dx && !world.dy) return;
    const commands: any[] = [];
    for (const { layer } of boxed) {
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
          <span class="align-glyph" aria-hidden="true">{@html api.ui.icon(GLYPHS[mode])}</span>
        </button>
      {/each}
    </div>
  {/each}
</div>

<style>
  /* Two wells, horizontal and vertical, each holding its three buttons. */
  .align-strip { display: flex; gap: 6px; margin: 2px 4px 10px; }
  .align-group { display: flex; flex: 1; height: 28px; border-radius: 7px; background: var(--bg-field); }
  button { display: flex; align-items: center; justify-content: center; flex: 1; height: 28px; padding: 0; border: 0; border-radius: 7px; background: transparent; color: color-mix(in srgb, var(--tx-label) 50%, var(--tx-2)); cursor: default; transition: background var(--dur-1), color var(--dur-1); }
  button:hover { background: rgb(var(--ink-rgb) / .06); color: var(--tx); }
  button:active { background: rgb(var(--ink-rgb) / .1); color: var(--tx); transform: scale(.94); transition: none; }
  button:focus-visible { outline: 2px solid var(--accent); outline-offset: -2px; }
</style>
