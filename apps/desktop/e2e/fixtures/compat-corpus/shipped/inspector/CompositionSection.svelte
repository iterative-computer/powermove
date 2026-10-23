<script lang="ts">
  import { inspectorContext, type EditBinding, type SelectOption } from './context';

  const { api, doc, mixed } = inspectorContext();
  const { ColorField, FillField, NumField, Row, Section, SelectField, ToggleField } = api.ui.controls;
  const { compositionBinding } = api.ui.controls.binding;

  type SceneParamView = {
    name: string;
    label: string;
    control: 'num' | 'color' | 'toggle' | 'select';
    value: unknown;
    min?: number;
    max?: number;
    options: SelectOption[];
  };

  const compEdit = (field: string, label: string): EditBinding =>
    compositionBinding(field as any, { label, origin: 'inspector' });
  const paramEdit = (param: SceneParamView): EditBinding => ({
    mode: 'command',
    label: param.label,
    origin: 'inspector',
    command: (value) => ({
      type: 'set_scene_parameter',
      name: param.name,
      value: value as any
    })
  });

  function normalizeOptions(value: unknown): SelectOption[] {
    if (!Array.isArray(value)) return [];
    return value.map((option) => {
      if (typeof option === 'string') return option;
      if (option && typeof option === 'object') {
        const source = option as { v?: unknown; label?: unknown };
        return { v: source.v, label: String(source.label ?? source.v ?? '') };
      }
      return String(option);
    });
  }

  function normalizeParam(source: any): SceneParamView | null {
    /* Legacy repaired malformed scene params in-place. This Phase 4 view is
       deliberately display-only; normalization does not mutate project data. */
    if (!source || (!source.label && !source.name)) return null;
    const control: SceneParamView['control'] = ['color', 'toggle', 'select'].includes(source.control)
      ? source.control
      : 'num';
    let value = source.value;
    if (value === undefined || (control === 'num' && !Number.isFinite(Number(value)))) {
      value = control === 'color' ? '#FF6B1A' : control === 'toggle' ? false : Number(source.min) || 0;
    }
    return {
      name: String(source.name),
      label: String(source.label || source.name),
      control,
      value,
      min: source.min == null ? undefined : Number(source.min),
      max: source.max == null ? undefined : Number(source.max),
      options: normalizeOptions(source.options)
    };
  }

  const params = $derived((doc.tick.structure, doc.tick.values, doc.proj,
    (Object.values(api.project.get().params ?? {}) as any[])
      .map(normalizeParam)
      .filter((param): param is SceneParamView => param !== null)));
</script>

<Section {api} title="Composition" />
<Row {api} label="Width"><NumField {api} {mixed} get={() => api.project.get().w} edit={compEdit('width', 'Width')} label="Width" step={2} min={16} /></Row>
<Row {api} label="Height"><NumField {api} {mixed} get={() => api.project.get().h} edit={compEdit('height', 'Height')} label="Height" step={2} min={16} /></Row>
<Row {api} label="Duration"><NumField {api} {mixed} get={() => api.project.get().dur} edit={compEdit('duration', 'Duration')} label="Duration" step={0.5} precision={2} unit="s" /></Row>
<Row {api} label="Frame rate"><NumField {api} {mixed} get={() => api.project.get().fps} edit={compEdit('fps', 'Frame rate')} label="Frame rate" step={1} /></Row>
<Row {api} label="Background"><FillField {api} get={() => api.project.get().backgroundFill} edit={compEdit('backgroundFill', 'Background fill')} label="Background fill" fallback={api.project.get().bg} /></Row>

{#if params.length}
  <Section {api} title="Scene parameters" />
  {#each params as param (param.name)}
    <Row {api} label={param.label}>
      {#if param.control === 'color'}
        <ColorField {api} {mixed} get={() => param.value} edit={paramEdit(param)} label={param.label} />
      {:else if param.control === 'toggle'}
        <ToggleField {api} {mixed} get={() => param.value} edit={paramEdit(param)} label={param.label} />
      {:else if param.control === 'select'}
        <SelectField {api} {mixed} get={() => param.value} edit={paramEdit(param)} options={param.options} label={param.label} />
      {:else}
        <NumField
          {api}
          {mixed}
          get={() => param.value}
          edit={paramEdit(param)}
          label={param.label}
          step={param.min != null && param.max != null ? (param.max - param.min) / 200 || 0.01 : 0.01}
          min={param.min}
          max={param.max}
          precision={3}
        />
      {/if}
    </Row>
  {/each}
{/if}

<div class="empty">Select a layer to edit its properties.</div>
