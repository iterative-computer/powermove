<script lang="ts">
  import { inspectorContext } from './context';
  import { type EditBinding } from '../../renderer/src/controls/gesture';
  import { axisContentKey, axisPath, type FontAxis } from 'powermove';
  const { api, doc, transport } = inspectorContext();
  const { NumField, Row } = api.ui.controls;
  let { PM, layer, axis }: { PM: Record<string, any>; layer: any; axis: Pick<FontAxis, 'tag' | 'label' | 'default'> & Partial<FontAxis> } = $props();
  const key = $derived(axisContentKey(axis.tag)), path = $derived(axisPath(axis.tag));
  const property = $derived((doc.tick.values, doc.tick.history, doc.proj, layer.d[key]));
  const animated = $derived((doc.tick.values, doc.tick.history, !!property?.kf?.length));
  const current = $derived((doc.tick.values, doc.tick.history, transport.time, !!property && !!PM.hasKeyAt?.(layer, property, transport.time)));
  const initial = $derived(axis.tag === 'wght' ? Math.max(axis.min ?? -Infinity, Math.min(axis.max ?? Infinity,
    Number(PM.resolveContent?.(layer, transport.time)?.weight ?? layer.d.weight) || axis.default)) : axis.default);
  const value = $derived((doc.tick.values, doc.tick.history, transport.time,
    property?.kf ? PM.evP(layer, property, transport.time, path) : initial));
  const step = $derived(axis.tag === 'ital' ? 1 : .01);
  const commands = (next: unknown, mode = 'auto'): any[] => [
    ...(!layer.d[key] ? [{ type: 'set_content', target: layer.id, patch: { [key]: PM.P(initial) } }] : []),
    { type: 'set_property', target: layer.id, path, value: Number(next), time: transport.time, mode, preserveHandEdits: false }
  ];
  const edit = $derived<EditBinding>({ mode: 'command', label: `${axis.label} axis`, origin: 'inspector', command: next => commands(next) });
  function toggle() {
    PM.Edit.apply(animated ? [
      { type: 'replace_keyframes', target: layer.id, path, keyframes: [], preserveHandEdits: false },
      { type: 'set_property', target: layer.id, path, value, mode: 'static', preserveHandEdits: false }
    ] : commands(value, 'keyframe'), { label: `${animated ? 'Remove animation from' : 'Animate'} ${axis.label}`, origin: 'inspector' });
    PM.TL?.reveal?.(layer, [path]); PM.invalidate();
  }
</script>

<div class="type-axis" data-font-axis={axis.tag} data-channel={path} title={`${axis.label} (${axis.tag})`}>
  <Row label={axis.label}>
    {#snippet left()}
      <button type="button" class="stopwatch property-stopwatch" class:on={animated} class:at-key={current}
        aria-label={`${animated ? 'Remove animation from' : 'Animate'} ${axis.label} · ${axis.tag}`}
        aria-pressed={animated} onclick={toggle}><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 3 20 12 12 21 4 12Z"/></svg></button>
    {/snippet}
    <NumField {PM} get={() => value} {edit} label={`${axis.label} axis`} min={axis.min} max={axis.max} {step} speed={axis.min != null && axis.max != null ? Math.max(.01, (axis.max - axis.min) / 300) / step : 50} precision={2} />
  </Row>

</div>
<style>
  .type-axis { padding-bottom: 5px; }
</style>
