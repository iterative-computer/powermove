<script lang="ts">
  import { untrack } from 'svelte';
  import type { ChannelValue, ControlEditBinding, Layer, PowermoveAPI } from 'powermove';
  import type { FieldSpec } from './scene-model';
  import type { SceneChannel } from './scene-types';
  import type { SceneUiState } from './scene-state.svelte';

  /* One keyframeable scene channel, laid out like the inspector's ChannelRow. */
  let { api, ui, layer, path, field, channel }: {
    api: PowermoveAPI;
    ui: SceneUiState;
    layer: Layer;
    path: string;
    field: FieldSpec;
    channel: SceneChannel | null;
  } = $props();
  const { Row, NumField, SliderField, ColorField, ToggleField } = untrack(() => api.ui.controls);

  const prop = $derived(channel as unknown as Parameters<PowermoveAPI['anim']['evP']>[1] | null);
  const value = $derived((ui.version, ui.time, prop ? api.anim.evP(layer, prop, ui.time, path) : null));
  const animated = $derived((ui.version, !!channel?.kf?.length));
  const atKey = $derived((ui.version, ui.time, !!prop && !!api.anim.hasKeyAt(layer, prop, ui.time)));
  const edit = $derived<ControlEditBinding>({
    mode: 'command',
    label: field.label,
    origin: 'inspector',
    command: (next: unknown) => ({
      type: 'set_property', target: layer.id, path, value: next as ChannelValue,
      time: api.transport.time(), mode:api.scene3d.getAutoKey()?'keyframe':'auto', preserveHandEdits: false,markIntent:'human'
    })
  });

  function toggleKey(event: MouseEvent): void {
    event.stopPropagation();
    if (!prop || value == null) return;
    const time = api.transport.time();
    const label = `${atKey ? 'Remove keyframe for' : 'Add keyframe for'} ${field.label}`;
    if (!animated) {
      api.edit.apply({ type: 'set_property', target: layer.id, path, value: value as ChannelValue, time, mode: 'keyframe', preserveHandEdits: false,markIntent:'human' }, { label, origin: 'inspector' });
    } else {
      if (atKey) {
        const key = api.anim.hasKeyAt(layer, prop, time);
        const keyframes = channel!.kf.filter(k => k !== key).map((k: any) => ({time:k.t,value:k.v,hold:!!k.hold,eo:k.eo,ei:k.ei,spring:k.spring}));
        api.edit.apply({type:'replace_keyframes',target:layer.id,path,keyframes,preserveHandEdits:false},{label,origin:'inspector'});
      } else {
        api.edit.apply({type:'set_property',target:layer.id,path,value:value as ChannelValue,time,mode:'keyframe',preserveHandEdits:false,markIntent:'human'},{label,origin:'inspector'});
      }
    }
    api.transport.invalidate();
  }
</script>

<div role="group" aria-label={`${field.label} property`} data-scene-channel={path}>
  <Row {api} label={field.label}>
    {#snippet left()}
      <button type="button" class="stopwatch property-stopwatch" class:on={animated} class:at-key={atKey}
        aria-label={`${atKey ? 'Remove keyframe for' : 'Add keyframe for'} ${field.label}`} aria-pressed={atKey}
        title={atKey ? 'Remove keyframe' : 'Add keyframe'} disabled={!prop} onclick={toggleKey}>
        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 3 20 12 12 21 4 12Z"/></svg>
      </button>
    {/snippet}
    {#if field.kind === 'color'}
      <ColorField {api} get={() => value} {edit} label={field.label} />
    {:else if field.kind === 'toggle'}
      <ToggleField {api} get={() => value} {edit} label={field.label} />
    {:else}
      {@const Field = field.min != null && field.max != null && field.max > field.min ? SliderField : NumField}
      <div class="well" data-prefix={field.prefix}>
        <Field angle={field.unit === '°'} {api} get={() => value} {edit} label={field.label} ariaLabel={field.label}
          step={field.step ?? 1} min={field.min} max={field.max} unit={field.unit} link={!!channel?.expr} />
      </div>
    {/if}
  </Row>
</div>
