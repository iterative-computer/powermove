<script lang="ts">
  import { untrack } from 'svelte';
  import type { ChannelValue, ControlEditBinding, Layer, PowermoveAPI } from 'powermove';
  import type { SceneUiState } from './scene-state.svelte';
  import { SURFACE_NAMES, installSurfaceProperty, surfaceChannel, surfacePath, type Surface, type SurfacePropertySpec } from './extrusion';

  /* One surface's Material Option, laid out exactly like a model material's
     (MaterialProperty): stopwatch, label with a plain-language tooltip, then
     the slider or color well. Until it's first changed the property follows
     the layer (its fill color, the default finish); the first edit installs it. */
  let { api, ui, layer, surface, spec }: { api: PowermoveAPI; ui: SceneUiState; layer: Layer; surface: Surface; spec: SurfacePropertySpec } = $props();
  const { Row, SliderField, ColorField } = untrack(() => api.ui.controls);

  const path = $derived(surfacePath(surface, spec.key));
  const channel = $derived((ui.version, surfaceChannel(layer, surface, spec.key)));
  const inherited = $derived((ui.version, ui.time, (api.scene3d.getSurfaceMaterialProperties(layer.id, surface, ui.time)?.values as any)?.[spec.key]));
  const value = $derived((ui.version, ui.time, channel ? api.anim.evP(layer, channel, ui.time, path) ?? inherited : inherited));
  const animated = $derived((ui.version, !!channel?.kf?.length));
  const atKey = $derived((ui.version, ui.time, !!channel && !!api.anim.hasKeyAt(layer, channel, ui.time)));
  const fullLabel = $derived(`${SURFACE_NAMES[surface]} ${spec.label}`);
  const edit = $derived<ControlEditBinding>({
    mode: 'command',
    label: fullLabel,
    origin: 'inspector',
    prepare: () => { if (!surfaceChannel(api.model.layer(layer.id), surface, spec.key)) installSurfaceProperty(api, layer.id, surface, spec.key, value); },
    command: (next: unknown) => ({ type: 'set_property', target: layer.id, path, value: next as ChannelValue, time: api.transport.time(), mode: 'auto', preserveHandEdits: false })
  });

  function toggleKey(event: MouseEvent): void {
    event.stopPropagation();
    if (value == null) return;
    const time = api.transport.time();
    const label = `${atKey ? 'Remove keyframe for' : 'Add keyframe for'} ${fullLabel}`;
    if (!channel) installSurfaceProperty(api, layer.id, surface, spec.key, value, time);
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

<div role="group" aria-label={`${fullLabel} property`} data-surface-property={path} title={spec.tip}>
  <Row {api} label={spec.label}>
    {#snippet left()}
      <button type="button" class="stopwatch property-stopwatch" class:on={animated} class:at-key={atKey}
        aria-label={`${atKey ? 'Remove keyframe for' : 'Add keyframe for'} ${fullLabel}`} aria-pressed={atKey}
        title={atKey ? 'Remove keyframe' : animated ? 'Add keyframe' : `Animate ${spec.label}`} onclick={toggleKey}>
        {@html api.ui.icon(atKey ? 'diamondFill' : 'diamond')}
      </button>
    {/snippet}
    {#if spec.kind === 'color'}
      <ColorField {api} get={() => value} {edit} label={fullLabel} />
    {:else}
      <div class="well">
        <SliderField {api} get={() => value} {edit} label={fullLabel} ariaLabel={fullLabel} step={spec.step ?? 1} min={spec.min} max={spec.max} unit={spec.unit} link={!!channel?.expr} />
      </div>
    {/if}
  </Row>
</div>
