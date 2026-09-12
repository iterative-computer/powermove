<script lang="ts">
  import { inspectorContext, type EditBinding } from './context';
  import AnimatedRow from './AnimatedRow.svelte';
  import ChannelRow from './ChannelRow.svelte';
  import PropertyStopwatch from './PropertyStopwatch.svelte';
  import { evaluatedValue, isProperty } from 'powermove';
  import Icon from './Icon.svelte';
  import { inspectorRefresh } from './refresh.svelte.js';

  const { api, doc, transport } = inspectorContext();
  const { Row, Section, SelectField, ToggleField } = api.ui.controls;

  let { PM, layer }: { PM: Record<string, any>; layer: any } = $props();

  const fields = [
    ['x', 'X', 1, 'px'], ['y', 'Y', 1, 'px'],
    ['w', 'Width', 1, 'px'], ['h', 'Height', 1, 'px'],
    ['rotation', 'Rotation', 1, '°'], ['feather', 'Feather', 0.5, 'px']
  ] as const;
  const masks = $derived((inspectorRefresh.version, doc.tick.structure, doc.tick.values, doc.proj, [...(layer.masks ?? [])]));

  function mutate(label: string, operation: () => void): void {
    PM.hist.do(label, () => {
      operation();
      PM.touch();
    });
    PM.invalidate?.();
    inspectorRefresh.bump();
  }

  const enabled = (mask: any) => (doc.tick.values, doc.proj, transport.time, evaluatedValue(PM, layer, mask.on, transport.time, `m.${mask.id}.on`) !== false);

  function addMask(): void {
    mutate('Add mask', () => layer.masks.push(PM.mkMask('rect', PM.curComp())));
  }

  function propertyBinding(maskId: string | number, key: string, label: string): EditBinding {
    return {
      mode: 'command',
      label,
      origin: 'inspector',
      command: (value) => ({
        type: 'set_property',
        target: layer.id,
        path: `m.${maskId}.${key}`,
        value: value as any,
        time: transport.time,
        mode: 'auto',
        preserveHandEdits: false
      })
    };
  }

</script>

<Section title="Masks" />
{#if masks.length === 0}
  <button type="button" class="chip wide" aria-label="Add mask"  onclick={addMask}>
    <Icon name="plus" />Add mask
  </button>
{:else}
  {#each masks as mask, index (mask.id ?? index)}
    {@const maskId = mask.id ?? index}
    <div class="row mask-head" style="margin-top:4px;background:var(--ink-1)" data-mask-id={maskId}>
      <span class="twirl open" aria-hidden="true"><Icon name="chev" /></span>
      <div class="k" style="color:var(--tx);font-weight:500">Mask {index + 1}</div>
      <button
        type="button"
        class:on={enabled(mask)}
        class="stopwatch"
        aria-label={`${enabled(mask) ? 'Disable' : 'Enable'} mask ${index + 1}`}
        aria-pressed={enabled(mask)}
        onclick={() => PM.Edit.apply({ type: 'set_property', target: layer.id, path: `m.${maskId}.on`, value: !enabled(mask), time: transport.time, mode: 'auto', preserveHandEdits: false }, { label: 'Toggle mask', origin: 'inspector' })}
      ><Icon name="eye" /></button>
      <button
        type="button"
        class="stopwatch"
        title="Delete mask"
        aria-label={`Delete mask ${index + 1}`}
        onclick={() => mutate('Remove mask', () => { layer.masks.splice(index, 1); })}
      ><Icon name="x" /></button>
    </div>
    <div class="grp mask-params">
      <AnimatedRow {PM} {layer} path={`m.${maskId}.on`} label="Enabled">
        <ToggleField {PM} get={() => enabled(mask)} edit={propertyBinding(maskId, 'on', 'Enable mask')} label="Enabled" />
      </AnimatedRow>
      <Row label="Shape">
        {#snippet left()}<PropertyStopwatch {PM} {layer} path={`m.${maskId}.shape`} label="Mask shape" fallback={mask.shape} />{/snippet}
        <SelectField
          {PM}
          get={() => (doc.tick.values, doc.proj, transport.time, isProperty(mask.shape) ? PM.evP(layer, mask.shape, transport.time, `m.${maskId}.shape`) : mask.shape)}
          edit={propertyBinding(maskId, 'shape', 'Mask shape')}
          options={PM.MASK_SHAPES}
          label="Shape"
        />
      </Row>
      <Row label="Mode">
        {#snippet left()}<PropertyStopwatch {PM} {layer} path={`m.${maskId}.mode`} label="Mask mode" fallback={mask.mode} />{/snippet}
        <SelectField
          {PM}
          get={() => (doc.tick.values, doc.proj, transport.time, isProperty(mask.mode) ? PM.evP(layer, mask.mode, transport.time, `m.${maskId}.mode`) : mask.mode)}
          edit={propertyBinding(maskId, 'mode', 'Mask mode')}
          options={['add', 'subtract']}
          label="Mode"
        />
      </Row>
      {#each fields as [key, label, step] (key)}
        {@const property = mask.p[key]}
        <ChannelRow {PM} {layer} channel={`m.${maskId}.${key}`} {label} {property} {step}
          getValue={(time) => PM.evP(layer, property, time, key)} />
      {/each}
    </div>
  {/each}
  <button type="button" class="chip wide" aria-label="Add mask" onclick={addMask}>
    <Icon name="plus" />Add mask
  </button>
{/if}
