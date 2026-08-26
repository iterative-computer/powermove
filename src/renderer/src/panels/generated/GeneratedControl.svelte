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

  let {
    PM,
    control,
    get,
    edit,
    toolState,
    onPreview,
    onReset
  }: {
    PM: Record<string, any>;
    control: GeneratedControl;
    get?: () => unknown;
    edit?: EditBinding;
    toolState: Record<string, unknown>;
    onPreview: (preview: PreviewModel | null) => void;
    onReset: () => void;
  } = $props();
</script>

{#if control.type === 'readout'}
  <Row label={control.label}><Readout {PM} source={control.source} /></Row>
{:else if control.type === 'button'}
  <GeneratedButton {PM} {control} {toolState} {onPreview} {onReset} />
{:else if get && edit && control.type === 'curve'}
  <CurveControl {PM} {control} {get} {edit} />
{:else if get && edit && control.type === 'text'}
  <Row label={control.label}><TextField {PM} {get} {edit} label={control.label} mono={false} /></Row>
{:else if get && edit && control.type === 'color'}
  <Row label={control.label}><ColorField {PM} {get} {edit} label={control.label} /></Row>
{:else if get && edit && control.type === 'fill'}
  <Row label={control.label}><FillField {PM} {get} {edit} label={control.label} fallback={PM.proj.bg} /></Row>
{:else if get && edit && control.type === 'toggle'}
  <Row label={control.label}><ToggleField {PM} {get} {edit} label={control.label} /></Row>
{:else if get && edit && control.type === 'select'}
  <Row label={control.label}><SelectField {PM} {get} {edit} label={control.label} options={control.options} /></Row>
{:else if get && edit && control.type === 'slider'}
  <Row label={control.label}>
    <NumField
      {PM}
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
