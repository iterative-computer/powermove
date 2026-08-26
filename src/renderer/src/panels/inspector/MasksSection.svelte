<script lang="ts">
  import { doc } from '../../state/document.svelte';
  import { transport } from '../../state/transport.svelte';
  import NumField from '../../controls/NumField.svelte';
  import Row from '../../controls/Row.svelte';
  import Section from '../../controls/Section.svelte';
  import SelectField from '../../controls/SelectField.svelte';
  import type { EditBinding } from '../../controls/gesture';
  import LegacyIcon from './LegacyIcon.svelte';
  import { inspectorRefresh } from './refresh.svelte';

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

  function addMask(): void {
    mutate('Add mask', () => layer.masks.push(PM.mkMask('rect', PM.curComp())));
  }

  function directBinding(label: string, write: (value: unknown) => void): EditBinding {
    return {
      mode: 'set',
      label,
      set(value) {
        write(value);
        PM.touch();
        PM.invalidate?.();
        inspectorRefresh.bump();
      }
    };
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

  function toggleKeys(property: any, key: string, label: string): void {
    mutate(`Animate ${label}`, () => {
      if (property.kf.length) {
        property.v = PM.evP(layer, property, PM.time, key);
        property.kf = [];
      } else {
        PM.setKeyOn(property, PM.time - layer.from, property.v, 'power', PM.proj.fps);
      }
    });
  }
</script>

<Section title="Masks" />
{#if masks.length === 0}
  <button type="button" class="chip wide" aria-label="Add mask"  onclick={addMask}>
    <LegacyIcon {PM} name="plus" />Add mask
  </button>
{:else}
  {#each masks as mask, index (mask.id ?? index)}
    {@const maskId = mask.id ?? index}
    <div class="row mask-head" style="margin-top:4px;background:var(--ink-1)" data-mask-id={maskId}>
      <span class="twirl open" aria-hidden="true"><LegacyIcon {PM} name="chev" /></span>
      <div class="k" style="color:var(--tx);font-weight:500">Mask {index + 1}</div>
      <button
        type="button"
        class:on={mask.on !== false}
        class="stopwatch"
        aria-label={`${mask.on === false ? 'Enable' : 'Disable'} mask ${index + 1}`}
        aria-pressed={mask.on !== false}
        onclick={() => mutate('Toggle mask', () => { mask.on = mask.on === false; })}
      ><LegacyIcon {PM} name="eye" /></button>
      <button
        type="button"
        class="stopwatch"
        title="Delete mask"
        aria-label={`Delete mask ${index + 1}`}
        onclick={() => mutate('Remove mask', () => { layer.masks.splice(index, 1); })}
      ><LegacyIcon {PM} name="x" /></button>
    </div>
    <div class="grp mask-params">
      <Row label="Shape">
        <SelectField
          {PM}
          get={() => (doc.tick.values, doc.proj, mask.shape)}
          edit={directBinding('Mask shape', (value) => { mask.shape = value; })}
          options={PM.MASK_SHAPES}
          label="Shape"
        />
      </Row>
      <Row label="Mode">
        <SelectField
          {PM}
          get={() => (doc.tick.values, doc.proj, mask.mode || 'add')}
          edit={directBinding('Mask mode', (value) => { mask.mode = value; })}
          options={['add', 'subtract']}
          label="Mode"
        />
      </Row>
      {#each fields as [key, label, step] (key)}
        {@const property = mask.p[key]}
        <Row {label}>
          {#snippet left()}
            <button
              type="button"
              class:on={property.kf.length > 0}
              class="stopwatch"
              aria-label={`Animate ${label}`}
              aria-pressed={property.kf.length > 0}
              onclick={() => toggleKeys(property, key, label)}
            ><LegacyIcon {PM} name="clock" /></button>
          {/snippet}
          <NumField
            {PM}
            get={() => (doc.tick.values, doc.proj, transport.time, PM.evP(layer, property, transport.time, key))}
            edit={propertyBinding(maskId, key, label)}
            {label}
            {step}
          />
        </Row>
      {/each}
    </div>
  {/each}
  <button type="button" class="chip wide" aria-label="Add mask" onclick={addMask}>
    <LegacyIcon {PM} name="plus" />Add mask
  </button>
{/if}
