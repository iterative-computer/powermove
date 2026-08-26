<script lang="ts">
  import { doc } from '../../state/document.svelte';
  import { transport } from '../../state/transport.svelte';
  import ColorField from '../../controls/ColorField.svelte';
  import Row from '../../controls/Row.svelte';
  import Section from '../../controls/Section.svelte';
  import type { EditBinding } from '../../controls/gesture';
  import ChannelRow from './ChannelRow.svelte';
  import LegacyIcon from './LegacyIcon.svelte';
  import { showFxMenu } from './actions';
  import { inspectorRefresh } from './refresh.svelte';

  let { PM, layer }: { PM: Record<string, any>; layer: any } = $props();

  let openVersion = $state(0);
  const effects = $derived((inspectorRefresh.version, doc.tick.structure, doc.proj, [...(layer.fx ?? [])]));

  function propertyEdit(path: string, label: string): EditBinding {
    return {
      mode: 'command',
      label,
      origin: 'inspector',
      command: (value) => ({
        type: 'set_property',
        target: layer.id,
        path,
        value: value as any,
        time: transport.time,
        preserveHandEdits: false
      })
    };
  }

  function isOpen(effect: any): boolean {
    openVersion;
    return !!PM.UIState.getFxOpen(effect);
  }

  function toggleOpen(effect: any): void {
    PM.UIState.setFxOpen(effect, !isOpen(effect));
    openVersion++;
  }

  function setEnabled(event: MouseEvent, effect: any): void {
    event.stopPropagation();
    PM.Edit.apply(
      { type: 'set_effect', target: layer.id, effect: effect.id, patch: { enabled: !effect.on } },
      { label: 'Toggle effect', origin: 'inspector' }
    );
    PM.invalidate?.();
  }

  function remove(event: MouseEvent, effect: any): void {
    event.stopPropagation();
    PM.Edit.apply(
      { type: 'remove_effect', target: layer.id, effect: effect.id },
      { label: 'Remove effect', origin: 'inspector' }
    );
    PM.invalidate?.();
  }
</script>

<Section title="Effects" />
{#if effects.length === 0}
  <button
    type="button"
    class="chip wide"
    onpointerdown={(event) => {
      event.preventDefault();
      showFxMenu(PM, event.currentTarget, layer);
    }}
  ><LegacyIcon {PM} name="plus" />Add effect</button>
{/if}

{#each effects as effect (effect.id)}
  {@const definition = PM.FX?.[effect.type]}
  {#if definition}
    {@const expanded = isOpen(effect)}
    {@const paramsId = `fx-params-${layer.id}-${effect.id}`}
    <div
      class="row fx-head"
      style="margin-top:4px;background:var(--ink-1)"
      data-effect-id={effect.id}
    >
      <button
        type="button"
        class="fx-expand"
        aria-label={`${expanded ? 'Collapse' : 'Expand'} ${definition.label}`}
        aria-expanded={expanded}
        aria-controls={paramsId}
        onclick={() => toggleOpen(effect)}
      >
        <span class:open={expanded} class="twirl" aria-hidden="true"><LegacyIcon {PM} name="chev" /></span>
        <span class="k" style="color:var(--tx);font-weight:500;text-align:left">{definition.label}</span>
      </button>
      <button
        type="button"
        class:on={effect.on}
        class="stopwatch"
        aria-label={`${effect.on ? 'Disable' : 'Enable'} ${definition.label}`}
        aria-pressed={!!effect.on}
        onclick={(event) => setEnabled(event, effect)}
      ><LegacyIcon {PM} name="eye" /></button>
      <button
        type="button"
        class="stopwatch"
        title="Remove effect"
        aria-label={`Remove ${definition.label}`}
        onclick={(event) => remove(event, effect)}
      ><LegacyIcon {PM} name="x" /></button>
    </div>

    {#if expanded}
      <div class="grp fx-params" id={paramsId}>
        {#each definition.params ?? [] as parameter (parameter.k)}
          {@const property = effect.p?.[parameter.k]}
          {#if property}
            {#if parameter.type === 'color'}
              <Row label={parameter.label}>
                <ColorField
                  {PM}
                  get={() => (doc.tick.values, doc.proj, property.v)}
                  edit={propertyEdit(`${effect.id}.${parameter.k}`, parameter.label)}
                  label={parameter.label}
                />
              </Row>
            {:else}
              <ChannelRow
                {PM}
                {layer}
                channel={`${effect.id}.${parameter.k}`}
                label={parameter.label}
                {property}
                getValue={(time) => PM.evP(layer, property, time, parameter.k)}
                step={parameter.step}
                min={parameter.min}
                max={parameter.max}
                unit={parameter.unit}
                showDiamond={false}
              />
            {/if}
          {/if}
        {/each}
      </div>
    {/if}
  {/if}
{/each}

<style>
  .fx-expand {
    display: flex;
    flex: 1;
    align-items: center;
    align-self: stretch;
    min-width: 0;
    gap: 8px;
    text-align: left;
  }
</style>
