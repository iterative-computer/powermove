<script lang="ts">
  import {
    currentParam,
    ensureParam,
    sceneParameterCommand,
    sourceBinding,
    type PreviewModel
  } from '../core/generated-bindings';
  import type { GeneratedControl, GeneratedSection } from '../core/types/workspace';
  import { EditGesture, type EditBinding } from '../controls/gesture';
  import { sel } from '../state/selection.svelte';
  import Disclosure from '../controls/Disclosure.svelte';
  import RampField from '../controls/RampField.svelte';
  import PointField from '../controls/PointField.svelte';
  import Row from '../controls/Row.svelte';
  import { doc } from '../state/document.svelte';
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
  const api = PM.Kernel.api('generated-controls');

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

  const controls: PreparedControl[] = section.controls.map(prepare);
  let primaryCount = 0;
  const secondary = new Set(controls.filter(({control})=>{
    if (control.type === 'button' || control.type === 'readout') return control.advanced === true;
    const advanced = control.advanced ?? primaryCount >= 5;
    if (!advanced) primaryCount++;
    return advanced;
  }).map(item=>item.key));

  const name = (item: PreparedControl) => String(item.control.stateKey || item.control.param || item.control.label).replace(/[^a-z0-9]/gi,'').toLowerCase();
  const startColor = controls.find(item=>item.control.type==='color'&&name(item)==='startcolor');
  const endColor = controls.find(item=>item.control.type==='color'&&name(item)==='endcolor');
  const pointPairs = controls.filter(item=>item.control.type==='slider'&&name(item).endsWith('x')).flatMap(x=>{
    const y=controls.find(item=>item.control.type==='slider'&&name(item)===name(x).slice(0,-1)+'y');
    if (!y || !x.get || !y.get || !x.edit || !y.edit || x.edit.mode !== y.edit.mode) return [];
    const cx=x.control as any, cy=y.control as any;
    if (!(cx.max>cx.min&&cy.max>cy.min)) return [];
    return [{x,y,label:x.control.label.replace(/\s*X$/i,''),advanced:secondary.has(x.key)||secondary.has(y.key)}];
  });
  const paired = new Set(pointPairs.flatMap(pair=>[pair.x.key,pair.y.key]));
  const secondaryChanged = $derived((doc.tick.values, doc.proj, controls.filter(item=>secondary.has(item.key)).some(item=>item.get && JSON.stringify(item.get())!==JSON.stringify(defaults[String(item.control.stateKey ?? '')] ?? item.control.def))));
  function pointEdit(x: PreparedControl, y: PreparedControl): EditBinding {
    const bindings=[x.edit!,y.edit!];
    if (bindings.every(binding=>binding.mode==='command')) return {mode:'command',label:'Move position',origin:'generated-ui',command:(next:any)=>bindings.flatMap((binding:any,index)=>typeof binding.command==='function'?binding.command(next[index]):{...binding.command,value:next[index]})};
    return {mode:'local',label:'Move position',set:(next:any)=>bindings.forEach((binding,index)=>new EditGesture(api,binding).write(next[index]))};
  }

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
  {#snippet field(prepared: PreparedControl)}
    <div data-generated-key={prepared.key}><GeneratedControlView {PM} {api} control={prepared.control} get={prepared.get} edit={prepared.edit} {toolState} onPreview={(next) => preview = next} onReset={reset} /></div>
  {/snippet}
  {#snippet point(pair: typeof pointPairs[number])}
    {@const x=pair.x.control as any}{@const y=pair.y.control as any}
    <Row {api} label={pair.label}><PointField {api} label={pair.label} get={()=>[Number(pair.x.get!()),Number(pair.y.get!())]} edit={pointEdit(pair.x,pair.y)} xEdit={pair.x.edit!} yEdit={pair.y.edit!} xLabel={pair.x.control.label} yLabel={pair.y.control.label} minX={x.min} maxX={x.max} minY={y.min} maxY={y.max} step={x.step} unit={x.unit} /></Row>
  {/snippet}
  {#if startColor?.get && endColor?.get && !secondary.has(startColor.key) && !secondary.has(endColor.key)}
    <RampField {api} label="Gradient" get={()=>[{id:'start',color:String(startColor.get!()),position:0},{id:'end',color:String(endColor.get!()),position:100}]} onSelect={(id)=>document.querySelector<HTMLElement>(`[data-svelte-panel="${CSS.escape(panelId)}"] [data-generated-key="${CSS.escape(id==='start'?startColor.key:endColor.key)}"] .color-field`)?.click()} />
  {/if}
  {#each pointPairs.filter(pair=>!pair.advanced) as pair}{@render point(pair)}{/each}
  {#each controls.filter(item=>!secondary.has(item.key)&&!paired.has(item.key)) as prepared (prepared.key)}{@render field(prepared)}{/each}
  {#if secondary.size}
    <Disclosure count={secondary.size} changed={secondaryChanged} remember={`generated:${section.id}`}>
      {#each pointPairs.filter(pair=>pair.advanced) as pair}{@render point(pair)}{/each}
      {#each controls.filter(item=>secondary.has(item.key)&&!paired.has(item.key)) as prepared (prepared.key)}{@render field(prepared)}{/each}
    </Disclosure>
  {/if}
  <PreviewList {preview} />
</div>
