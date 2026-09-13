<script lang="ts">
  import { inspectorContext } from './context';
  import { axisContentKey, axisPath, type FontAxis } from 'powermove';
  const { api, doc, transport, controlProps, edit: inspectorEdit, timeline } = inspectorContext();
  const { NumField, Row } = api.ui.controls;
  type EditBinding = ConstructorParameters<typeof api.ui.gesture>[0];
  let { layer, axis }: { layer: any; axis: Pick<FontAxis, 'tag' | 'label' | 'default'> & Partial<FontAxis> } = $props();
  const key = $derived(axisContentKey(axis.tag)), path = $derived(axisPath(axis.tag));
  const property = $derived((doc.tick.values, doc.tick.history, doc.proj, layer.d[key]));
  const animated = $derived((doc.tick.values, doc.tick.history, !!property?.kf?.length));
  const current = $derived((doc.tick.values, doc.tick.history, transport.time, !!property && !!api.anim.hasKeyAt?.(layer, property, transport.time)));
  const initial = $derived(axis.tag === 'wght' ? Math.max(axis.min ?? -Infinity, Math.min(axis.max ?? Infinity,
    Number((api.anim.resolveContent(layer, transport.time) as Record<string, any>).weight ?? layer.d.weight) || axis.default)) : axis.default);
  const value = $derived((doc.tick.values, doc.tick.history, transport.time,
    property?.kf ? api.anim.evP(layer, property, transport.time, path) : initial));
  const step = $derived(axis.tag === 'ital' ? 1 : .01);
  const commands = (next: unknown, mode = 'auto'): any[] => [
    ...(!layer.d[key] ? [{ type: 'set_content', target: layer.id, patch: { [key]: api.model.P(initial) } }] : []),
    { type: 'set_property', target: layer.id, path, value: Number(next), time: transport.time, mode, preserveHandEdits: false }
  ];
  const edit = $derived<EditBinding>({ mode: 'command', label: `${axis.label} axis`, origin: 'inspector', command: next => commands(next) });
  function toggle() {
    if (property?.kf) {
      api.history.do(current ? 'Remove keyframe' : 'Add keyframe', () => {
        const at = api.anim.hasKeyAt(layer, property, transport.time);
        if (at) api.anim.removeKey(property, at);
        else api.anim.setKeyOn(property, transport.time - layer.from, value!, 'linear', api.project.get().fps);
      });
    } else inspectorEdit.apply(commands(value, 'keyframe'), { label: `Add keyframe for ${axis.label}`, origin: 'inspector' });
    timeline()?.reveal(layer, [path]); api.transport.invalidate();
  }
</script>

<div class="type-axis" data-font-axis={axis.tag} data-channel={path} title={`${axis.label} (${axis.tag})`}>
  <Row label={axis.label}>
    {#snippet left()}
      <button type="button" class="stopwatch property-stopwatch" class:on={animated} class:at-key={current}
        aria-label={`${current ? 'Remove keyframe for' : 'Add keyframe for'} ${axis.label} · ${axis.tag}`}
        aria-pressed={current} onclick={toggle}><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 3 20 12 12 21 4 12Z"/></svg></button>
    {/snippet}
    <NumField {...controlProps} get={() => value} {edit} label={`${axis.label} axis`} min={axis.min} max={axis.max} {step} speed={axis.min != null && axis.max != null ? Math.max(.01, (axis.max - axis.min) / 300) / step : 50} precision={2} />
  </Row>

</div>
<style>
  .type-axis { padding-bottom: 5px; }
</style>
