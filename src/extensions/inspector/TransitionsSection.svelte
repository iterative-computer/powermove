<script lang="ts">
  import { inspectorContext, type EditBinding, type SelectOption } from './context';
  import ChannelRow from './ChannelRow.svelte';
  import Icon from './Icon.svelte';
  import { inspectorRefresh } from './refresh.svelte.js';

  const { api, doc, transport } = inspectorContext();
  const { ColorField, NumField, Row, Section, SelectField, ToggleField } = api.ui.controls;
  const { channelBinding } = api.ui.controls.binding;

  let { PM, layer }: { PM: Record<string, any>; layer: any } = $props();

  const edges = [
    { id: 'in', label: 'In', field: 'transitionIn' },
    { id: 'out', label: 'Out', field: 'transitionOut' }
  ] as const;

  const definitions = $derived.by(() => {
    inspectorRefresh.version;
    doc.tick.structure;
    doc.proj;

    const found = new Map<string, any>();
    try {
      for (const definition of PM.Kernel?.transitions?.list?.() ?? []) {
        if (definition?.id && !found.has(definition.id)) found.set(definition.id, definition);
      }
    } catch {
      // An extension registry should not make the rest of the inspector unusable.
    }
    for (const [id, definition] of Object.entries(PM.TRANSITIONS ?? {}) as Array<[string, any]>) {
      if (!found.has(id)) found.set(id, { id, ...definition });
    }
    return [...found.values()];
  });

  const typeOptions = $derived<SelectOption[]>([
    { v: null, label: 'None' },
    ...definitions.map((definition) => ({ v: definition.id, label: definition.label ?? definition.id }))
  ]);

  function transitionFor(field: 'transitionIn' | 'transitionOut'): any {
    doc.tick.structure;
    doc.tick.values;
    doc.proj;
    return layer[field] ?? null;
  }

  function definitionFor(type: string | undefined): any {
    return definitions.find((definition) => definition.id === type);
  }

  function setTransition(edge: 'in' | 'out', transition: Record<string, any> | null, label: string): void {
    PM.Edit.apply(
      { type: 'set_transition', layer: layer.id, edge, transition },
      { label, origin: 'inspector' }
    );
    PM.invalidate?.();
  }

  function typeBinding(edge: 'in' | 'out', edgeLabel: string): EditBinding {
    return {
      mode: 'command',
      label: `${edgeLabel} transition`,
      origin: 'inspector',
      command: (value) => ({
        type: 'set_transition',
        layer: layer.id,
        edge,
        transition: value == null ? null : { type: String(value) }
      } as any)
    };
  }

  function parameterValues(transition: any, definition: any): Record<string, number | string | boolean> {
    return Object.fromEntries((definition?.params ?? []).flatMap((parameter: any) => {
      const property = transition?.p?.[parameter.k];
      if (!property) return [];
      const value = PM.evP(layer, property, transport.time, parameter.k);
      return [[parameter.k, value]];
    }));
  }

  function durationBinding(
    edge: 'in' | 'out',
    edgeLabel: string,
    transition: any,
    definition: any
  ): EditBinding {
    return {
      mode: 'command',
      label: `${edgeLabel} transition duration`,
      origin: 'inspector',
      command: (value) => ({
        type: 'set_transition',
        layer: layer.id,
        edge,
        transition: {
          type: transition.type,
          dur: Number(value),
          p: parameterValues(transition, definition)
        }
      } as any)
    };
  }

  function paramBinding(field: 'transitionIn' | 'transitionOut', key: string, label: string): EditBinding {
    return channelBinding(PM, layer.id, `${field}.p.${key}`, {
      label,
      origin: 'inspector',
      time: () => transport.time
    });
  }
</script>

<Section title="Transitions" />
{#each edges as edge (edge.id)}
  {@const transition = transitionFor(edge.field)}
  {@const definition = definitionFor(transition?.type)}
  <div class="transition-edge" data-transition-edge={edge.id}>
    <div class="transition-head">
      <Row label={edge.label}>
        <div class="transition-type">
          <SelectField
            {PM}
            get={() => transitionFor(edge.field)?.type ?? null}
            edit={typeBinding(edge.id, edge.label)}
            options={typeOptions}
            label={`${edge.label} transition type`}
          />
          {#if transition}
            <button
              type="button"
              class="stopwatch"
              title={`Remove ${edge.label.toLowerCase()} transition`}
              aria-label={`Remove ${edge.label.toLowerCase()} transition`}
              onclick={() => setTransition(edge.id, null, `Remove ${edge.label.toLowerCase()} transition`)}
            ><Icon name="x" /></button>
          {/if}
        </div>
      </Row>
    </div>

    {#if transition && definition}
      <div class="grp transition-params" role="group" aria-label={`${edge.label} transition settings`}>
        <Row label="Duration">
          <NumField
            {PM}
            get={() => (doc.tick.values, doc.proj, transitionFor(edge.field)?.dur ?? .5)}
            edit={durationBinding(edge.id, edge.label, transition, definition)}
            label={`${edge.label} transition duration`}
            min={.02}
            max={600}
            step={.05}
            unit="s"
          />
        </Row>

        {#each definition.params ?? [] as parameter (parameter.k)}
          {@const property = transition.p?.[parameter.k]}
          {#if property}
            {@const path = `${edge.field}.p.${parameter.k}`}
            {#if parameter.type === 'color'}
              <Row label={parameter.label}>
                <ColorField
                  {PM}
                  get={() => (doc.tick.values, doc.proj, property.v)}
                  edit={paramBinding(edge.field, parameter.k, parameter.label)}
                  label={`${edge.label} transition ${parameter.label}`}
                />
              </Row>
            {:else if parameter.type === 'toggle'}
              <Row label={parameter.label}>
                <ToggleField
                  {PM}
                  get={() => (doc.tick.values, doc.proj, property.v)}
                  edit={paramBinding(edge.field, parameter.k, parameter.label)}
                  label={`${edge.label} transition ${parameter.label}`}
                />
              </Row>
            {:else}
              <ChannelRow
                {PM}
                {layer}
                channel={path}
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
  </div>
{/each}

<style>
  .transition-edge {
    margin-top: 4px;
  }

  .transition-head {
    background: var(--ink-1);
  }

  .transition-type {
    display: flex;
    align-items: center;
    gap: 4px;
    min-width: 0;
  }
</style>
