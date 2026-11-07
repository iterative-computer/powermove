<script lang="ts">
  import { untrack } from 'svelte';
  import type { ChannelValue, ControlEditBinding, Layer, PowermoveAPI } from 'powermove';
  import type { SceneUiState } from './scene-state.svelte';
  import { installProperty, propertyChannel, propertyPath, type PropertySpec } from './materials';

  /* One intrinsic material property (After Effects' Material Options): a
     stopwatch, the label with a plain-language tooltip, and the app's slider
     or color well. Until it's first changed the property inherits (from the
     whole model, then the model file); the first edit installs it. */
  let { api, ui, layer, index, spec }: { api: PowermoveAPI; ui: SceneUiState; layer: Layer; index: number | null; spec: PropertySpec } = $props();
  const { Row, SliderField, ColorField } = untrack(() => api.ui.controls);

  const path = $derived(propertyPath(index, spec.key));
  const channel = $derived((ui.version, propertyChannel(layer, index, spec.key)));
  const inherited = $derived((ui.version, ui.time, api.scene3d.getMaterialProperties(layer.id, index, ui.time)?.values[spec.key]));
  const value = $derived((ui.version, ui.time, channel ? api.anim.evP(layer, channel, ui.time, path) ?? inherited : inherited));
  const animated = $derived((ui.version, !!channel?.kf?.length));
  const atKey = $derived((ui.version, ui.time, !!channel && !!api.anim.hasKeyAt(layer, channel, ui.time)));
  const edit = $derived<ControlEditBinding>({
    mode: 'command',
    label: spec.label,
    origin: 'inspector',
    // The first change installs the property at its inherited value, then edits normally.
    prepare: () => { if (!propertyChannel(api.model.layer(layer.id), index, spec.key)) installProperty(api, layer.id, index, spec.key, value); },
    command: (next: unknown) => ({ type: 'set_property', target: layer.id, path, value: next as ChannelValue, time: api.transport.time(), mode: 'auto', preserveHandEdits: false })
  });

  function toggleKey(event: MouseEvent): void {
    event.stopPropagation();
    if (value == null) return;
    const time = api.transport.time();
    const label = `${atKey ? 'Remove keyframe for' : 'Add keyframe for'} ${spec.label}`;
    if (!channel) installProperty(api, layer.id, index, spec.key, value, time);
    else if (!animated) api.edit.apply({ type: 'set_property', target: layer.id, path, value: value as ChannelValue, time, mode: 'keyframe', preserveHandEdits: false }, { label, origin: 'inspector' });
    else api.history.do(label, () => {
      const key = api.anim.hasKeyAt(layer, channel, time);
      if (key) api.anim.removeKey(channel, key);
      else api.anim.setKeyOn(channel, time - layer.from, value as ChannelValue, 'linear', api.project.get().fps);
      api.anim.touch();
    });
    api.transport.invalidate();
  }
</script>

<div role="group" aria-label={`${spec.label} property`} data-material-property={path} title={spec.tip}>
  <Row {api} label={spec.label}>
    {#snippet left()}
      <button type="button" class="stopwatch property-stopwatch" class:on={animated} class:at-key={atKey}
        aria-label={`${atKey ? 'Remove keyframe for' : 'Add keyframe for'} ${spec.label}`} aria-pressed={atKey}
        title={atKey ? 'Remove keyframe' : animated ? 'Add keyframe' : `Animate ${spec.label}`} onclick={toggleKey}>
        {@html api.ui.icon(atKey ? 'diamondFill' : 'diamond')}
      </button>
    {/snippet}
    {#if spec.kind === 'color'}
      <ColorField {api} get={() => value} {edit} label={spec.label} />
    {:else}
      <div class="well">
        <SliderField {api} get={() => value} {edit} label={spec.label} ariaLabel={spec.label} step={spec.step ?? 1} min={spec.min} max={spec.max} unit={spec.unit} link={!!channel?.expr} />
      </div>
    {/if}
  </Row>
</div>
