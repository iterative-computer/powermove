<script lang="ts">
  /* A shape's stroke: width (per side on rectangles, behind a toggle like
     independent corners), paint, and — once there is a stroke — its style:
     position, dashes, caps, joins. Rows appear only where they change the
     drawing: caps on open paths or dashes, miter limit where corners are
     sharp enough to reach it. */
  import AnimatedRow from './AnimatedRow.svelte';
  import Choice from './Choice.svelte';
  import PaintRows from './PaintRows.svelte';
  import { inspectorContext, type EditBinding } from './context';
  import { cloneRaw, contentCommand, DASH_CAPS, END_CAPS, JOINS, propertyCommand, rawAt, STROKE_POSITIONS } from './figma-data';
  import { editableSelection, perLayerEdit } from './multi-edit';

  let { layer, content }: { layer: any; content: Record<string, any> } = $props();
  const { api, transport, mixed, edit: inspectorEdit } = inspectorContext();
  const { NumField, SelectField } = api.ui.controls;
  const { contentBinding } = api.ui.controls.binding;

  const style = $derived((content.strokeStyle && typeof content.strokeStyle === 'object' ? content.strokeStyle : {}) as Record<string, any>);
  const shape = $derived(String(content.shape ?? 'rect'));
  const width = $derived(shape==='line'?Math.max(1,Number(content.stroke)||6):Math.max(0, Number(content.stroke) || 0));
  const paths = $derived((void content, (Array.isArray(layer.d?.paths) ? layer.d.paths : []) as any[]));
  const closedValue = (path: any) => {
    const closed = path?.p?.closed ?? path?.closed;
    return !!(closed && typeof closed === 'object' && 'v' in closed ? closed.v : closed);
  };
  const openPath = $derived(shape === 'line' || paths.some((path) => !closedValue(path)));
  const dashes = $derived((Array.isArray(style.dashes) ? style.dashes : []).map(Number));
  const dash = $derived(dashes[0] ?? 0);
  const gap = $derived(dashes[1] ?? dashes[0] ?? 0);
  const sides = $derived(Array.isArray(style.weights) && style.weights.length === 4 ? style.weights.map(Number) : null);
  const rect = $derived(shape === 'rect' && !paths.length);
  const sharp = $derived(paths.length > 0 || shape === 'polygon' || shape === 'star');
  const join = $derived(String(style.join ?? 'miter'));
  const SIDES = [['Top', 0], ['Right', 1], ['Bottom', 2], ['Left', 3]] as const;
  const SIDE_GLYPHS = ['M3 3.5h10', 'M12.5 3v10', 'M3 12.5h10', 'M3.5 3v10'];

  const live = () => api.model.layer(layer.id) ?? layer;
  /* Whether a stored field exists (it has a channel to key); tracks content changes. */
  const has = (key: string) => (void content, rawAt(layer, key) !== undefined);
  /* A style field: a channel edit where it exists, otherwise each selected
     shape's style is created with it, from that shape's own data. */
  const DEFAULT_STYLE = { align: 'center', dashes: [], dashOffset: 0, cap: 'NONE', startCap: 'NONE', endCap: 'NONE', join: 'miter', miterLimit: 4 };
  const resolved = (target: any) => api.anim.resolveContent(target, transport.time) as Record<string, any>;
  function styleWith(target: any, patch: Record<string, unknown>) {
    const raw = cloneRaw(rawAt(target, 'strokeStyle')) as Record<string, unknown> | undefined;
    return { ...DEFAULT_STYLE, ...(resolved(target).shape==='line'?{cap:'ROUND',startCap:'ROUND',endCap:'ROUND'}:{}), ...(raw && typeof raw === 'object' ? raw : {}), ...patch };
  }
  const shapeEdit = (has: (target: any) => boolean, build: (target: any, exists: boolean) => any) =>
    perLayerEdit(api, live(), (target) => target.type === 'shape' && has(target), (target, exists) => target.type === 'shape' ? build(target, exists) : null);
  function field(key: string, label: string, map: (value: unknown) => unknown = (value) => value): EditBinding {
    return {
      mode: 'command', label, origin: 'inspector',
      command: (value) => shapeEdit((target) => rawAt(target, `strokeStyle.${key}`) !== undefined, (target, exists) => exists
        ? propertyCommand(target.id, `c.strokeStyle.${key}`, map(value), transport.time)
        : contentCommand(target.id, { strokeStyle: styleWith(target, { [key]: map(value) }) }))
    };
  }
  const number = (value: unknown) => Math.max(0, Number(value) || 0);
  const dashList = (target: any) => (resolved(target).strokeStyle?.dashes ?? []).map(Number) as number[];

  /* Dashes are a dash/gap list; zero dash means a solid stroke (an empty list). */
  const dashEdit: EditBinding = {
    mode: 'command', label: 'Stroke dash', origin: 'inspector',
    command: (value) => {
      const next = number(value);
      return shapeEdit((target) => next > 0 && dashList(target).length >= 1, (target, exists) => exists
        ? propertyCommand(target.id, 'c.strokeStyle.dashes.0', next, transport.time)
        : contentCommand(target.id, { strokeStyle: styleWith(target, { dashes: next > 0 ? [next, dashList(target)[1] || next] : [] }) }));
    }
  };
  const gapEdit: EditBinding = {
    mode: 'command', label: 'Stroke gap', origin: 'inspector',
    command: (value) => shapeEdit((target) => dashList(target).length >= 2, (target, exists) => exists
      ? propertyCommand(target.id, 'c.strokeStyle.dashes.1', number(value), transport.time)
      : contentCommand(target.id, { strokeStyle: styleWith(target, { dashes: [dashList(target)[0] || number(value), number(value)] }) }))
  };

  /* Turning sides on seeds every side from the current width, so nothing moves until a side is edited. */
  function toggleSides(): void {
    const on = !sides;
    const commands = editableSelection(api, live()).flatMap((target: any) => {
      const now = resolved(target), width = Math.max(0, Number(now.stroke) || 0);
      if (target.type !== 'shape' || now.shape !== 'rect' || target.d?.paths?.length || !!now.strokeStyle?.weights === on) return [];
      const next = styleWith(target, {}) as Record<string, unknown>;
      if (on) next.weights = [width, width, width, width];
      else delete next.weights;
      return [contentCommand(target.id, { strokeStyle: next })];
    });
    if (!commands.length) return;
    inspectorEdit.apply(commands, { label: 'Independent sides', origin: 'inspector' });
    api.transport.invalidate?.();
  }
  function sideEdit(index: number, label: string): EditBinding {
    return {
      mode: 'command', label, origin: 'inspector',
      command: (value) => editableSelection(api, live()).flatMap((target: any) => {
        const now = resolved(target), weights = now.strokeStyle?.weights;
        if (target.type !== 'shape' || !Array.isArray(weights) || rawAt(target, `strokeStyle.weights.${index}`) === undefined) return [];
        const next = number(value);
        const commands = [propertyCommand(target.id, `c.strokeStyle.weights.${index}`, next, transport.time)];
        // The drawing clips sides to the stroke's overall width; keep it the widest side.
        const widest = Math.max(next, ...weights.filter((_: unknown, i: number) => i !== index).map(Number));
        if (widest !== Number(now.stroke)) commands.push(contentCommand(target.id, { stroke: widest }));
        return commands;
      })
    };
  }
  const sideSummary = $derived(sides ? (sides.every((value) => value === sides[0]) ? `${sides[0]}px` : 'Mixed') : '');
</script>

<AnimatedRow {layer} label="Stroke" path={sides ? undefined : 'c.stroke'}>
  <div class="stroke-inline">
    {#if sides}
      <span class="stroke-readout" aria-label="Stroke width">{sideSummary}</span>
    {:else}
      <NumField {api} {mixed} get={() => width} edit={contentBinding(layer.id, 'stroke', { label: 'Stroke', origin: 'inspector' })} label="Stroke" step={0.5} min={0} unit="px" />
    {/if}
    {#if rect && width > 0}
      <button type="button" class="stroke-btn" class:on={!!sides} aria-pressed={!!sides} aria-label="Independent sides" title="Independent sides" onclick={toggleSides}>
        <svg viewBox="0 0 16 16" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" aria-hidden="true">
          <path d="M3.5 5.5v5M12.5 5.5v5M5.5 3.5h5M5.5 12.5h5" />
        </svg>
      </button>
    {/if}
  </div>
</AnimatedRow>
{#if sides}
  <div class="side-grid">
    {#each SIDES as [name, index] (name)}
      <div class="side-well" title={name}>
        <svg class="side-glyph" aria-hidden="true" viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round">
          <rect x="3" y="3" width="10" height="10" rx="1" opacity=".35" /><path d={SIDE_GLYPHS[index]} />
        </svg>
        <NumField {api} {mixed} get={() => sides?.[index] ?? 0} edit={sideEdit(index, `${name} stroke`)} label={`${name} stroke`} step={0.5} min={0} />
      </div>
    {/each}
  </div>
{/if}

<PaintRows {layer} {content} kind={shape==='line'?'fill':'stroke'} label="Stroke color" />

{#if width > 0}
  {#if shape !== 'line'}
  <AnimatedRow {layer} label="Position" path={has('strokeStyle.align') ? 'c.strokeStyle.align' : undefined}>
    <Choice value={String(style.align ?? 'center')} options={STROKE_POSITIONS} label="Stroke position" edit={field('align', 'Stroke position')} />
  </AnimatedRow>
  {/if}
  <AnimatedRow {layer} label="Dash" path={dashes.length ? 'c.strokeStyle.dashes.0' : undefined}>
    <NumField {api} {mixed} get={() => dash} edit={dashEdit} label="Stroke dash" step={1} min={0} unit="px" />
  </AnimatedRow>
  {#if dashes.length}
    <AnimatedRow {layer} label="Gap" path={dashes.length >= 2 ? 'c.strokeStyle.dashes.1' : undefined}>
      <NumField {api} {mixed} get={() => gap} edit={gapEdit} label="Stroke gap" step={1} min={0} unit="px" />
    </AnimatedRow>
    {#if !openPath}
      <AnimatedRow {layer} label="Dash caps" path={has('strokeStyle.cap') ? 'c.strokeStyle.cap' : undefined}>
        <SelectField {api} {mixed} get={() => String(style.cap ?? 'NONE').toUpperCase()} edit={field('cap', 'Dash caps')} options={DASH_CAPS} label="Dash caps" />
      </AnimatedRow>
    {/if}
  {/if}
  {#if openPath}
    <AnimatedRow {layer} label="Start" path={has('strokeStyle.startCap') ? 'c.strokeStyle.startCap' : undefined}>
      <SelectField {api} {mixed} get={() => String(style.startCap ?? style.cap ?? (shape==='line'?'ROUND':'NONE')).toUpperCase()} edit={field('startCap', 'Start cap')} options={END_CAPS} label="Start cap" />
    </AnimatedRow>
    <AnimatedRow {layer} label="End" path={has('strokeStyle.endCap') ? 'c.strokeStyle.endCap' : undefined}>
      <SelectField {api} {mixed} get={() => String(style.endCap ?? style.cap ?? (shape==='line'?'ROUND':'NONE')).toUpperCase()} edit={field('endCap', 'End cap')} options={END_CAPS} label="End cap" />
    </AnimatedRow>
  {/if}
  {#if shape !== 'line' && (shape !== 'ellipse' || paths.length)}
    <AnimatedRow {layer} label="Join" path={has('strokeStyle.join') ? 'c.strokeStyle.join' : undefined}>
      <Choice value={join} options={JOINS} label="Stroke join" edit={field('join', 'Stroke join')} />
    </AnimatedRow>
    {#if join === 'miter' && sharp}
      <AnimatedRow {layer} label="Miter limit" path={has('strokeStyle.miterLimit') ? 'c.strokeStyle.miterLimit' : undefined}>
        <NumField {api} {mixed} get={() => Number(style.miterLimit ?? 4)} edit={field('miterLimit', 'Miter limit', (value) => Math.max(1, Number(value) || 1))} label="Miter limit" step={0.5} min={1} precision={1} />
      </AnimatedRow>
    {/if}
  {/if}
{/if}

<style>
  .stroke-inline { flex: 1; min-width: 0; height: 100%; display: flex; align-items: center; gap: 2px; }
  .stroke-inline > :global(*:first-child) { flex: 1 1 auto; min-width: 0; }
  .stroke-readout {
    flex: 1; min-width: 0; height: 28px; display: flex; align-items: center; padding-left: 8px;
    border-radius: var(--r-sm); background: var(--bg-row); color: var(--tx-3); font-size: var(--fs-md);
  }
  .stroke-btn {
    width: 28px; height: 28px; flex: none; padding: 0;
    display: inline-flex; align-items: center; justify-content: center;
    border: 0; border-radius: var(--r-sm); background: transparent; color: var(--tx-2); cursor: pointer;
    transition: background var(--dur-1), color var(--dur-1);
  }
  .stroke-btn:hover { background: rgb(var(--ink-rgb) / .08); color: var(--tx); }
  .stroke-btn.on { background: color-mix(in srgb, var(--accent) 22%, transparent); color: var(--accent-tx); }
  .stroke-btn:focus-visible { outline: 2px solid var(--accent); outline-offset: -2px; }
  /* As Corner radius's independent corners: a 2x2 grid with glyphs in the wells. */
  .side-grid {
    display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 8px;
    margin: 4px calc(-1 * var(--pad)); padding: 0 16px;
    transform-origin: top center; animation: pm-menu-in .18s cubic-bezier(.23, 1, .32, 1) both;
  }
  .side-well { position: relative; min-width: 0; display: flex; align-items: center; }
  .side-glyph { position: absolute; left: 7px; color: var(--tx-3); pointer-events: none; }
  .side-well :global(input.num:not([type=range]):not([type=color]):not([type=file])) { padding-left: 26px; }
  @media (prefers-reduced-motion: reduce) { .side-grid { animation: none; } }
</style>
