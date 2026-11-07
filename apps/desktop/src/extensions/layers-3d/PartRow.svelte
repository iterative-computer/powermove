<script lang="ts">
  import { untrack } from 'svelte';
  import type { ChannelValue, ControlEditBinding, EditCommand, Layer, PowermoveAPI } from 'powermove';
  import type { SceneUiState } from './scene-state.svelte';
  import { partChannel, installPart, type PartField } from './parts';

  /* One part property (Position, Rotation, Scale) laid out like Transform:
     a diamond that keys every axis together, then one field per axis in the
     same well. Until a part is first changed it has no channels; the first
     edit installs them, at neutral values, in the same Undo step. */
  let { api, ui, layer, part, label, fields, unit = '', step = 1, min, version }: {
    api: PowermoveAPI; ui: SceneUiState; layer: Layer; part: string; label: string;
    fields: readonly PartField[]; unit?: string; step?: number; min?: number; version: number;
  } = $props();
  const { Row, NumField } = untrack(() => api.ui.controls);

  const channels = $derived((ui.version, version, fields.map(field => partChannel(api.model.layer(layer.id), part, field))));
  const values = $derived((ui.version, ui.time, version, api.scene3d.getPart(layer.id, part, ui.time).values));
  const animated = $derived(channels.some(channel => !!channel?.kf?.length));
  const atKey = $derived((ui.time, channels.length > 0 && channels.every(channel => !!channel && !!api.anim.hasKeyAt(layer, channel, ui.time))));
  const axisLabel = (field: PartField) => fields.length === 1 ? label : `${label} ${field.slice(-1).toUpperCase()}`;
  const path = (field: PartField) => `pt.${part}.${field}`;

  const edit = (field: PartField): ControlEditBinding => ({
    mode: 'command',
    label: axisLabel(field),
    origin: 'inspector',
    prepare: () => { if (!partChannel(api.model.layer(layer.id), part, field)) installPart(api, layer.id, part); },
    command: (next: unknown) => ({ type: 'set_property', target: layer.id, path: path(field), value: next as ChannelValue, time: api.transport.time(), mode: 'auto', preserveHandEdits: false })
  });

  function toggleKey(event: MouseEvent): void {
    event.stopPropagation();
    const time = api.transport.time(), text = `${atKey ? 'Remove keyframe for' : 'Add keyframe for'} ${label}`;
    const current = api.scene3d.getPart(layer.id, part, time).values as Record<string, unknown>;
    if (!channels.every(Boolean)) {
      // First touch: install the part with a key on these axes, in one step.
      const t = Math.max(0, time - layer.from);
      const result = api.scene3d.setPart(layer.id, part, Object.fromEntries(fields.map(field => [field, { v: current[field], kf: [{ t, v: current[field] }], expr: null }])), { label: text, origin: 'inspector' });
      if (!result.ok) api.ui.toast(result.message, { error: true });
    } else if (!animated) {
      const commands: EditCommand[] = fields.map(field => ({ type: 'set_property', target: layer.id, path: path(field), value: current[field] as ChannelValue, time, mode: 'keyframe', preserveHandEdits: false }));
      api.edit.apply(commands, { label: text, origin: 'inspector' });
    } else api.history.do(text, () => {
      for (const [index, field] of fields.entries()) {
        const channel = channels[index]!, key = api.anim.hasKeyAt(layer, channel, time);
        if (atKey) { if (key) api.anim.removeKey(channel, key); }
        else if (!key) api.anim.setKeyOn(channel, time - layer.from, current[field] as ChannelValue, 'linear', api.project.get().fps);
      }
      api.anim.touch();
    });
    api.transport.invalidate();
  }
</script>

<div role="group" aria-label={`${label} property`} data-part-property={fields.join(',')}>
  <Row {api} {label} pair={fields.length > 1}>
    {#snippet left()}
      <button type="button" class="stopwatch property-stopwatch" class:on={animated} class:at-key={atKey}
        aria-label={`${atKey ? 'Remove keyframe for' : 'Add keyframe for'} ${label}`} aria-pressed={atKey}
        title={atKey ? 'Remove keyframe' : animated ? 'Add keyframe' : `Animate ${label}`} onclick={toggleKey}>
        {@html api.ui.icon(atKey ? 'diamondFill' : 'diamond')}
      </button>
    {/snippet}
    {#each fields as field (field)}
      <div class="well" data-axis={fields.length > 1 ? field.slice(-1).toUpperCase() : undefined} data-part-field={field}>
        <NumField angle={unit === '°'} {api} get={() => (values as Record<string, unknown>)[field]} edit={edit(field)}
          label={axisLabel(field)} ariaLabel={axisLabel(field)} {step} {min} {unit} link={!!channels[fields.indexOf(field)]?.expr} />
      </div>
    {/each}
  </Row>
</div>
