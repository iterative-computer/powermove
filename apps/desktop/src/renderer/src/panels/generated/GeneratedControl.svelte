<script lang="ts">
  import ColorField from '../../controls/ColorField.svelte';
  import FillField from '../../controls/FillField.svelte';
  import NumField from '../../controls/NumField.svelte';
  import Row from '../../controls/Row.svelte';
  import SelectField from '../../controls/SelectField.svelte';
  import TextField from '../../controls/TextField.svelte';
  import ToggleField from '../../controls/ToggleField.svelte';
  import type { EditBinding } from '../../controls/gesture';
  import type { PreviewModel } from '../../core/generated-bindings';
  import type { GeneratedControl } from '../../core/types/workspace';
  import CurveControl from './CurveControl.svelte';
  import GeneratedButton from './GeneratedButton.svelte';
  import Readout from './Readout.svelte';
  import type { PowermoveAPI } from '../../kernel/api';

  let {
    PM,
    api,
    control,
    get,
    edit,
    toolState,
    onPreview,
    onReset
  }: {
    PM: Record<string, any>;
    api: PowermoveAPI;
    control: GeneratedControl;
    get?: () => unknown;
    edit?: EditBinding;
    toolState: Record<string, unknown>;
    onPreview: (preview: PreviewModel | null) => void;
    onReset: () => void;
  } = $props();
</script>

{#if control.type === 'readout'}
  <Row {api} label={control.label}><Readout {PM} source={control.source} /></Row>
{:else if control.type === 'button'}
  <GeneratedButton {PM} {control} {toolState} {onPreview} {onReset} />
{:else if get && edit && control.type === 'curve'}
  <CurveControl {PM} {api} {control} {get} {edit} />
{:else if get && edit && control.type === 'text'}
  <Row {api} label={control.label}><TextField {api} {get} {edit} label={control.label} mono={false} /></Row>
{:else if get && edit && control.type === 'color'}
  <Row {api} label={control.label}><ColorField {api} {get} {edit} label={control.label} /></Row>
{:else if get && edit && control.type === 'fill'}
  <Row {api} label={control.label}><FillField {api} {get} {edit} label={control.label} fallback={api.project.get().bg} /></Row>
{:else if get && edit && control.type === 'toggle'}
  <Row {api} label={control.label}><ToggleField {api} {get} {edit} label={control.label} /></Row>
{:else if get && edit && control.type === 'select'}
  <Row {api} label={control.label}><SelectField {api} {get} {edit} label={control.label} options={control.options} /></Row>
{:else if get && edit && control.type === 'slider'}
  <Row {api} label={control.label}>
    <NumField
      {api}
      {get}
      {edit}
      label={control.label}
      min={control.min}
      max={control.max}
      step={control.step || ((control.max - control.min) / 100) || .01}
      precision={3}
      unit={control.unit}
    />
  </Row>
{/if}
