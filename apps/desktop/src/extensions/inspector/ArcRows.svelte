<script lang="ts">
  /* An ellipse's arc, as in Figma: Start rotates the arc, Sweep is how much
     of the circle it covers, and Ratio cuts the hole for a donut. Stored as
     start/end radians and an inner radius from 0 to 1. */
  import AnimatedRow from './AnimatedRow.svelte';
  import { inspectorContext, type EditBinding } from './context';
  import { contentCommand, propertyCommand, rawAt } from './figma-data';
  import { editableSelection } from './multi-edit';

  let { layer, content }: { layer: any; content: Record<string, any> } = $props();
  const { api, transport, mixed } = inspectorContext();
  const { NumField } = api.ui.controls;
  const TAU = Math.PI * 2;

  const arc = $derived((content.arc && typeof content.arc === 'object' ? content.arc : { start: 0, end: TAU, inner: 0 }) as { start: number; end: number; inner: number });
  const start = $derived(Number(arc.start) || 0);
  const sweep = $derived((Number(arc.end ?? TAU) || 0) - start);
  const round = (value: number, digits = 1) => Math.round(value * 10 ** digits) / 10 ** digits;
  const live = () => api.model.layer(layer.id) ?? layer;
  const arcOf = (target: any) => (api.anim.resolveContent(target, transport.time) as Record<string, any>).arc ?? { start: 0, end: TAU, inner: 0 };
  const has = (key: string) => (void content, rawAt(layer, `arc.${key}`) !== undefined);

  /* Every field writes channels once the arc exists; the first edit creates
     it. Each selected ellipse computes from its own arc. */
  function write(values: (arc: { start: number; end: number; inner: number }) => Partial<Record<'start' | 'end' | 'inner', number>>) {
    const ellipse = (target: any) => target.type === 'shape' && (api.anim.resolveContent(target, transport.time) as any).shape === 'ellipse';
    return editableSelection(api, live()).flatMap((target: any) => {
      if (!ellipse(target)) return [];
      const now = arcOf(target), next = values(now);
      if (rawAt(target, 'arc') === undefined) return [contentCommand(target.id, { arc: { start: now.start ?? 0, end: now.end ?? TAU, inner: now.inner ?? 0, ...next } })];
      return Object.entries(next).map(([key, value]) => propertyCommand(target.id, `c.arc.${key}`, value, transport.time));
    });
  }
  const startEdit: EditBinding = {
    mode: 'command', label: 'Arc start', origin: 'inspector',
    command: (value) => {
      const next = (Number(value) || 0) * Math.PI / 180;
      // Start turns the arc; its sweep stays.
      return write((now) => ({ start: next, end: next + ((Number(now.end) || 0) - (Number(now.start) || 0)) }));
    }
  };
  const sweepEdit: EditBinding = {
    mode: 'command', label: 'Arc sweep', origin: 'inspector',
    command: (value) => {
      const fraction = Math.max(-1, Math.min(1, (Number(value) || 0) / 100));
      return write((now) => ({ end: (Number(now.start) || 0) + fraction * TAU }));
    }
  };
  const innerEdit: EditBinding = {
    mode: 'command', label: 'Arc ratio', origin: 'inspector',
    command: (value) => write(() => ({ inner: Math.max(0, Math.min(0.999, (Number(value) || 0) / 100)) }))
  };
</script>

<AnimatedRow {layer} label="Start" path={has('start') ? 'c.arc.start' : undefined}>
  <NumField {api} {mixed} get={() => round(start * 180 / Math.PI)} edit={startEdit} label="Arc start" step={1} unit="°" precision={1} />
</AnimatedRow>
<AnimatedRow {layer} label="Sweep" path={has('end') ? 'c.arc.end' : undefined}>
  <NumField {api} {mixed} get={() => round(sweep / TAU * 100)} edit={sweepEdit} label="Arc sweep" step={1} min={-100} max={100} unit="%" precision={1} />
</AnimatedRow>
<AnimatedRow {layer} label="Ratio" path={has('inner') ? 'c.arc.inner' : undefined}>
  <NumField {api} {mixed} get={() => round((Number(arc.inner) || 0) * 100)} edit={innerEdit} label="Arc inner radius ratio" step={1} min={0} max={99.9} unit="%" precision={1} />
</AnimatedRow>
