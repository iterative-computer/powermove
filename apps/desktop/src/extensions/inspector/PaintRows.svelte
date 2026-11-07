<script lang="ts">
  /* Fill or stroke paint for shapes: a solid color, or an editable gradient
     (type, stops, angle, scale, opacity). The gradient button beside the
     value switches between the two, like Corner radius's corner toggle. */
  import AnimatedRow from './AnimatedRow.svelte';
  import { inspectorContext, type EditBinding } from './context';
  import {
    colorAt, composeGradient, contentCommand, cloneRaw, defaultGradient, GRADIENT_LABELS, GRADIENT_TYPES,
    gradientGeometry, nextStopPosition, propertyCommand, rawAt, staticValue
  } from './figma-data';
  import { editableSelection } from './multi-edit';

  let { layer, content, kind = 'fill', label }: { layer: any; content: Record<string, any>; kind?: 'fill' | 'stroke'; label: string } = $props();
  const { api, doc, transport, mixed, edit: inspectorEdit } = inspectorContext();
  const { ColorField, NumField, RampField, Row, SelectField } = api.ui.controls;
  const { contentBinding } = api.ui.controls.binding;

  const root = $derived(kind === 'fill' ? 'fillGradient' : 'strokeGradient');
  const colorKey = $derived(kind === 'fill' ? 'color' : 'strokeColor');
  const gradient = $derived(content[root] && typeof content[root] === 'object' ? content[root] as { type: string; transform: number[]; stops: Array<{ position: number; color: string }>; opacity: number } : null);
  const stops = $derived(gradient?.stops ?? []);
  let chosen = $state(0);
  const selected = $derived(Math.max(0, Math.min(stops.length - 1, chosen)));
  const noun = $derived(kind === 'fill' ? 'Fill' : 'Stroke');

  /* Bindings read the live layer, so a drag keeps building on its last write. */
  const live = () => {
    const current = api.model.layer(layer.id) ?? layer;
    return { layer: current, content: api.anim.resolveContent(current, transport.time) as Record<string, any> };
  };
  const at = (key: string) => `c.${root}.${key}`;
  const property = (path: string, label: string, map: (value: unknown) => unknown = (value) => value): EditBinding => ({
    mode: 'command', label, origin: 'inspector',
    command: (value) => propertyCommand(layer.id, path, map(value), transport.time)
  });

  const typeEdit = $derived(property(at('type'), `${noun} gradient type`));
  const opacityEdit = $derived(property(at('opacity'), `${noun} gradient opacity`, (value) => Number(value) / 100));
  const stopColorEdit = $derived(property(at(`stops.${selected}.color`), 'Stop color'));
  const stopPositionEdit = $derived(property(at(`stops.${selected}.position`), 'Stop position', (value) => Math.max(0, Math.min(1, Number(value) / 100))));
  const transformPaths = $derived([0, 1, 2, 3, 4, 5].map((index) => at(`transform.${index}`)));

  const geometry = $derived(gradient ? gradientGeometry(gradient.transform, gradient.type, Number(content.w) || 1, Number(content.h) || 1) : { angle: 0, scale: 1 });
  /* Angle and scale are a view of the paint's transform: an edit rewrites its
     six numbers, from each selected layer's own transform. */
  const resolved = (target: any) => api.anim.resolveContent(target, transport.time) as Record<string, any>;
  const shapes = () => editableSelection(api, live().layer).filter((target: any) => target.type === 'shape');
  function transformEdit(part: 'angle' | 'scale', label: string): EditBinding {
    return {
      mode: 'command', label, origin: 'inspector',
      command: (value) => shapes().flatMap((target: any) => {
        const now = resolved(target), paint = now[root];
        if (!paint || rawAt(target, root) == null) return [];
        const w = Number(now.w) || 1, h = Number(now.h) || 1;
        const current = gradientGeometry(paint.transform, paint.type, w, h);
        const angle = part === 'angle' ? Number(value) : current.angle;
        const scale = part === 'scale' ? Math.max(0.01, Number(value) / 100) : current.scale;
        return composeGradient(paint.transform, paint.type, w, h, angle, scale)
          .map((entry, index) => propertyCommand(target.id, at(`transform.${index}`), entry, transport.time));
      })
    };
  }
  const angleEdit = $derived(transformEdit('angle', `${noun} gradient angle`));
  const scaleEdit = $derived(transformEdit('scale', `${noun} gradient scale`));

  /* The strip: each stop's position as 0–100. A drag edits only moved stops. */
  const rampStops = () => stops.map((stop, index) => ({ id: `stop-${index}`, color: String(stop.color), position: Math.round(Number(stop.position) * 1000) / 10 }));
  const rampEdit: EditBinding = {
    mode: 'command', label: 'Move gradient stop', origin: 'inspector',
    command: (value) => {
      const now = (live().content[root]?.stops ?? []) as Array<{ position: number }>;
      return (value as Array<{ id: string; position: number }>).flatMap((stop) => {
        const index = Number(stop.id.slice(5));
        const position = Math.max(0, Math.min(1, stop.position / 100));
        return now[index] && Math.abs(Number(now[index]!.position) - position) > 1e-6
          ? [propertyCommand(layer.id, at(`stops.${index}.position`), position, transport.time)] : [];
      });
    }
  };

  /* Structural edits: each selected shape changes from its own paint. */
  function structural(build: (target: any, now: Record<string, any>) => Record<string, unknown> | null, label: string): void {
    const commands = shapes().flatMap((target: any) => {
      const patch = build(target, resolved(target));
      return patch ? [contentCommand(target.id, patch)] : [];
    });
    if (!commands.length) return;
    inspectorEdit.apply(commands, { label, origin: 'inspector' });
    api.transport.invalidate?.();
  }

  function toggleGradient(): void {
    if (gradient) {
      // Back to solid: keep the gradient's first color so the shape stays recognisable.
      structural((_, now) => {
        if (!now[root]) return null;
        const first = [...(now[root].stops ?? [])].sort((a: any, b: any) => a.position - b.position)[0];
        return { [root]: null, ...(first ? { [colorKey]: String(first.color) } : {}) };
      }, `${noun} solid color`);
    } else {
      chosen = 0;
      structural((_, now) => now[root] ? null : { [root]: defaultGradient(now[colorKey]) }, `${noun} gradient`);
    }
  }

  function addStop(): void {
    const index = selected;
    structural((target, now) => {
      const raw = cloneRaw(rawAt(target, root)) as any, current = now[root]?.stops ?? [];
      if (!raw?.stops) return null;
      const position = nextStopPosition(current, Math.min(index, current.length - 1));
      raw.stops.push({ position, color: colorAt(current, position) });
      return { [root]: raw };
    }, 'Add gradient stop');
    chosen = stops.length;
  }

  function removeStop(): void {
    const index = selected;
    structural((target) => {
      const raw = cloneRaw(rawAt(target, root)) as any;
      if (!raw?.stops || raw.stops.length <= 2) return null;
      raw.stops.splice(Math.min(index, raw.stops.length - 1), 1);
      return { [root]: raw };
    }, 'Remove gradient stop');
    chosen = Math.max(0, index - 1);
  }

  const typeOptions = GRADIENT_TYPES.map((type) => ({ v: type, label: GRADIENT_LABELS[type] }));
  const order = $derived([...stops.keys()].sort((a, b) => Number(stops[a]!.position) - Number(stops[b]!.position)));
  void doc;
</script>

<AnimatedRow {layer} {label} path={gradient ? undefined : `c.${colorKey}`}>
  <div class="paint-inline" data-paint={kind}>
    {#if gradient}
      <SelectField {api} {mixed} get={() => gradient?.type ?? 'linear'} edit={typeEdit} options={typeOptions} label={`${noun} gradient type`} />
    {:else}
      <ColorField {api} {mixed} get={() => content[colorKey] ?? '#808080'} edit={contentBinding(layer.id, colorKey, { label: kind === 'fill' ? 'Fill' : 'Stroke', origin: 'inspector' })} label={label} />
    {/if}
    <button type="button" class="paint-btn" class:on={!!gradient} aria-pressed={!!gradient}
      aria-label={`${noun} gradient`} title={gradient ? 'Use a solid color' : 'Use a gradient'} onclick={toggleGradient}>
      <svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true">
        <rect x="2.5" y="2.5" width="11" height="11" rx="2.5" fill="none" stroke="currentColor" stroke-width="1.3" />
        <rect x="4.5" y="4.5" width="2" height="7" rx=".5" fill="currentColor" />
        <rect x="7" y="4.5" width="2" height="7" rx=".5" fill="currentColor" opacity=".6" />
        <rect x="9.5" y="4.5" width="2" height="7" rx=".5" fill="currentColor" opacity=".3" />
      </svg>
    </button>
  </div>
</AnimatedRow>

{#if gradient}
  <Row {api} label="Stops">
    <div class="paint-stops" data-gradient-stops={kind}>
      <RampField {api} label={`${noun} gradient`} get={rampStops} edit={rampEdit} selected={`stop-${selected}`} onSelect={(id: string) => { chosen = Number(id.slice(5)); }} />
      <button type="button" class="paint-btn" aria-label={`Add ${noun.toLowerCase()} gradient stop`} title="Add stop" onclick={addStop}>
        <svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true"><path d="M8 3.5v9M3.5 8h9" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" /></svg>
      </button>
    </div>
  </Row>
  {#key `${root}:${selected}`}
    <AnimatedRow {layer} label="Stop color" path={at(`stops.${selected}.color`)}>
      <ColorField {api} {mixed} get={() => stops[selected]?.color ?? '#000000'} edit={stopColorEdit} label={`Stop ${order.indexOf(selected) + 1} color`} />
    </AnimatedRow>
    <AnimatedRow {layer} label="Stop position" path={at(`stops.${selected}.position`)}>
      <div class="paint-inline">
        <NumField {api} {mixed} get={() => Math.round(Number(stops[selected]?.position ?? 0) * 1000) / 10} edit={stopPositionEdit} label={`Stop ${order.indexOf(selected) + 1} position`} step={1} min={0} max={100} unit="%" />
        <button type="button" class="paint-btn" aria-label={`Remove ${noun.toLowerCase()} gradient stop`} title="Remove stop" disabled={stops.length <= 2} onclick={removeStop}>
          <svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true"><path d="M3.5 8h9" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" /></svg>
        </button>
      </div>
    </AnimatedRow>
  {/key}
  <AnimatedRow {layer} label="Angle" path={transformPaths[0]} paths={transformPaths}>
    <NumField {api} {mixed} get={() => geometry.angle} edit={angleEdit} label={`${noun} gradient angle`} step={1} unit="°" precision={1} />
  </AnimatedRow>
  <AnimatedRow {layer} label="Scale" path={transformPaths[0]} paths={transformPaths}>
    <NumField {api} {mixed} get={() => Math.round(geometry.scale * 1000) / 10} edit={scaleEdit} label={`${noun} gradient scale`} step={1} min={1} unit="%" precision={1} />
  </AnimatedRow>
  <AnimatedRow {layer} label="Opacity" path={at('opacity')}>
    <NumField {api} {mixed} get={() => Math.round(Number(staticValue(gradient?.opacity) ?? 1) * 1000) / 10} edit={opacityEdit} label={`${noun} gradient opacity`} step={1} min={0} max={100} unit="%" />
  </AnimatedRow>
{/if}

<style>
  .paint-inline { flex: 1; min-width: 0; height: 100%; display: flex; align-items: center; gap: 2px; }
  .paint-inline > :global(*:first-child) { flex: 1 1 auto; min-width: 0; }
  .paint-stops { flex: 1; min-width: 0; display: flex; align-items: center; gap: 2px; }
  /* The strip sits in the value column; its stop handles hang below the bar. */
  .paint-stops { padding: 2px 0 10px; }
  .paint-stops :global(.ramp-field) { flex: 1; min-width: 0; padding: 0 7px 0 6px; }
  .paint-stops :global(.ramp-bar) { height: 22px; }
  .paint-btn {
    width: 28px; height: 28px; flex: none; padding: 0;
    display: inline-flex; align-items: center; justify-content: center;
    border: 0; border-radius: var(--r-sm); background: transparent; color: var(--tx-2); cursor: pointer;
    transition: background var(--dur-1), color var(--dur-1);
  }
  .paint-btn:hover:not(:disabled) { background: rgb(var(--ink-rgb) / .08); color: var(--tx); }
  .paint-btn:disabled { opacity: .35; cursor: default; }
  .paint-btn.on { background: color-mix(in srgb, var(--accent) 22%, transparent); color: var(--accent-tx); }
  .paint-btn:focus-visible { outline: 2px solid var(--accent); outline-offset: -2px; }
</style>
