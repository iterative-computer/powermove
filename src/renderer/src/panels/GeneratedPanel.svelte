<script lang="ts">
  import {
    currentParam,
    ensureParam,
    sceneParameterCommand,
    sourceBinding,
    type PreviewModel
  } from '../core/generated-bindings';
  import type { GeneratedControl, GeneratedSection } from '../core/types/workspace';
  import type { EditBinding } from '../controls/gesture';
  import { sel } from '../state/selection.svelte';
  import GeneratedControlView from './generated/GeneratedControl.svelte';
  import PreviewList from './generated/PreviewList.svelte';
  import type { PanelProps } from './registerSveltePanel';

  type PreparedControl = {
    control: GeneratedControl;
    get?: () => unknown;
    edit?: EditBinding;
    key: string;
  };

  let {
    panelId,
    PM: suppliedPM,
    section: suppliedSection
  }: PanelProps & {
    PM?: Record<string, any>;
    section?: GeneratedSection;
  } = $props();

  /* Panel builds are immutable mount records; activate/mutate remounts them. */
  const initial = () => {
    const PM = suppliedPM ?? (window as any).PM;
    const section = suppliedSection ?? PM.WS?.current?.custom?.find((item: GeneratedSection) => item.id === panelId);
    return { PM, section, initialPanelId: panelId };
  };
  const { PM, section, initialPanelId } = initial();
  if (!section) throw new Error(`Generated panel manifest not found: ${initialPanelId}`);

  let toolState = $state<Record<string, unknown>>({ ...(section.state || {}) });
  const defaults: Record<string, unknown> = $state.snapshot(toolState);
  let preview = $state<PreviewModel | null>(null);
  let saveTimer = 0;

  function persistState(): void {
    (section as any).state = $state.snapshot(toolState);
    window.clearTimeout(saveTimer);
    saveTimer = window.setTimeout(() => {
      saveTimer = 0;
      PM.WS.save();
    }, 140);
  }

  function localSet(key: string, value: unknown): void {
    toolState[key] = value;
    persistState();
    preview = null;
  }

  function prepare(control: GeneratedControl, index: number): PreparedControl {
    if (control.type === 'button' || control.type === 'readout') {
      return { control, key: `${String((control as any).id || control.label)}-${index}` };
    }
    const binding = sourceBinding(PM, control);
    const local = !!control.stateKey;
    const parameter = binding || local ? null : ensureParam(PM, control);
    const authored = control as unknown as Record<string, any>;
    const target = authored.target || authored.binding?.target;
    const selectionTarget = target === '$selection' || target === 'selection';
    const get = binding
      ? () => {
          // The read is intentionally inside the getter used by the field's
          // derived value: selection changes update it without remounting.
          if (selectionTarget) void sel.layers;
          return binding.get();
        }
      : local
        ? () => toolState[control.stateKey] ?? control.def
        : () => currentParam(PM, parameter!).value;
    const edit: EditBinding = local
      ? { mode: 'local', label: control.label, set: (value) => localSet(control.stateKey, value) }
      : binding
        ? { mode: 'command', label: control.label, origin: 'generated-ui', command: binding.command }
        : {
            mode: 'command',
            label: control.label,
            origin: 'generated-ui',
            command: (value) => sceneParameterCommand(parameter!, value)
          };
    return {
      control,
      get,
      edit,
      key: `${String((control as any).id || control.stateKey || control.label)}-${index}`
    };
  }

  const controls = section.controls.map(prepare);

  function reset(): void {
    for (const key of Object.keys(toolState)) delete toolState[key];
    Object.assign(toolState, defaults);
    persistState();
    preview = null;
  }
</script>

<div class="insp" data-svelte-panel={panelId}>
  {#if section.note}
    <div style="color:var(--tx-3);font-size:var(--fs-sm);line-height:1.6;padding:2px 4px 8px">{section.note}</div>
  {/if}
  {#each controls as prepared (prepared.key)}
    <GeneratedControlView
      {PM}
      control={prepared.control}
      get={prepared.get}
      edit={prepared.edit}
      {toolState}
      onPreview={(next) => preview = next}
      onReset={reset}
    />
  {/each}
  <PreviewList {preview} />
</div>
