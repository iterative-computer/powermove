<script lang="ts">
  import AnimatedRow from './AnimatedRow.svelte';
  import { resolveContent } from 'powermove';
  import { onDestroy } from 'svelte';
  import { inspectorContext, type EditBinding, type SelectOption } from './context';
  import TypeSettings from './TypeSettings.svelte';
  import TextAlignment from './TextAlignment.svelte';
  import { axisContentKey } from 'powermove';

  const { api, doc, transport } = inspectorContext();
  const { ColorField, FontField, NumField, Section, SelectField } = api.ui.controls;
  const { contentBinding } = api.ui.controls.binding;

  let {
    PM,
    layer,
    fontsVersion = 0
  }: {
    PM: Record<string, any>;
    layer: any;
    fontsVersion?: number;
  } = $props();

  const content = $derived((doc.tick.values, doc.proj, transport.time, resolveContent(PM, layer, transport.time)));
  const shape = $derived((doc.tick.values, doc.proj, content.shape));
  const assets = $derived((doc.tick.assets, doc.tick.structure, doc.proj, Object.values(PM.proj?.assets ?? {}) as any[]));
  const shaderError = $derived((doc.tick.values, doc.proj,
    layer.type === 'shader' ? PM.GL?.compileError?.(PM.UIState?.getShaderMeta?.(layer)?.shaderKey) : ''));
  let editingText = false;
  let hasVariableWeight = $state(false);

  const get = (key: string, fallback?: unknown) => () => content[key] == null ? fallback : content[key];
  const edit = (key: string, label: string): EditBinding =>
    contentBinding(PM, layer.id, key, { label, origin: 'inspector' });

  function beginText(): void {
    if (editingText) return;
    PM.Edit.begin('Edit text', { origin: 'inspector' });
    editingText = true;
  }

  function inputText(event: Event): void {
    if (!editingText) beginText();
    PM.Edit.dispatch({
      type: 'set_content',
      target: layer.id,
      patch: { text: (event.currentTarget as HTMLTextAreaElement).value }
    });
    PM.invalidate?.('render');
  }

  function commitText(): void {
    if (!editingText) return;
    editingText = false;
    PM.Edit.commit('Edit text');
  }

  onDestroy(commitText);

  function mediaOptions(kind: 'image' | 'video'): SelectOption[] {
    return assets
      .filter((asset) => asset?.kind === kind)
      .map((asset) => ({ v: asset.id, label: String(asset.name) }))
      .concat([{ v: null, label: 'none' }]);
  }


</script>

{#if !(layer.type === 'shape' && layer.d.paths?.length)}<Section title="Content" />{/if}

{#if layer.type === 'text'}
  <AnimatedRow {PM} {layer} label="Text" path="c.text">
  <textarea
    data-inspector-text-layer={layer.id}
    aria-label="Text"
    value={String(content.text ?? '')}
    style="width:100%;min-height:54px;background:var(--bg-row);border-radius:var(--r-sm);padding:8px 10px;font-size:var(--fs-md);line-height:1.5;resize:vertical;color:var(--tx)"
    onfocus={beginText}
    oninput={inputText}
    onblur={commitText}
    onkeydown={(event) => event.stopPropagation()}
  ></textarea>
  </AnimatedRow>
  <AnimatedRow {PM} {layer} label="Font" path="c.font">
    {#key fontsVersion}
      <FontField
        {PM}
        get={get('font', '')}
        edit={edit('font', 'Font')}
        label="Font"
        weight={() => Number(content.weight) || 400}
      />
    {/key}
  </AnimatedRow>
  {#if !hasVariableWeight && content[axisContentKey('wght')] == null}
    {@const weights = [...new Set([Number(content.weight) || 400, 100, 200, 300, 400, 500, 600, 700, 800, 900])].sort((a, b) => a - b)}
    <AnimatedRow {PM} {layer} label="Weight" path="c.weight">
      <SelectField
        {PM}
        get={get('weight', 400)}
        edit={edit('weight', 'Weight')}
        options={weights.map((value) => ({ v: value, label: String(value) }))}
        label="Weight"
        onChange={(value: unknown) => PM.Fonts?.ensure?.(content.font, Number(value) || 400)}
      />
    </AnimatedRow>
  {/if}
  <AnimatedRow {PM} {layer} label="Size" path="c.size"><NumField {PM} get={get('size', 0)} edit={edit('size', 'Size')} label="Size" step={1} min={4} unit="px" /></AnimatedRow>
  <AnimatedRow {PM} {layer} label="Tracking" path="c.tracking"><NumField {PM} get={get('tracking', 0)} edit={edit('tracking', 'Tracking')} label="Tracking" step={0.5} unit="px" /></AnimatedRow>
  <AnimatedRow {PM} {layer} label="Leading" path="c.leading"><NumField {PM} get={get('leading', 0)} edit={edit('leading', 'Leading')} label="Leading" step={0.02} precision={2} /></AnimatedRow>
  <AnimatedRow {PM} {layer} label="Align" path="c.align"><TextAlignment {layer} value={String(content.align ?? 'center')} /></AnimatedRow>
  <AnimatedRow {PM} {layer} label="Color" path="c.color"><ColorField {PM} get={get('color', '#F2F2F2')} edit={edit('color', 'Text color')} label="Text color" /></AnimatedRow>
  <TypeSettings {PM} {layer} family={String(content.font ?? '')} onVariableWeight={(value) => { hasVariableWeight = value; }} />
{:else if (layer.type === 'solid' || layer.type === 'shape') && !layer.d.paths?.length}
  <AnimatedRow {PM} {layer} label="Fill" path="c.color"><ColorField {PM} get={get('color', '#808080')} edit={edit('color', 'Fill')} label="Fill" /></AnimatedRow>
  {#if layer.type === 'shape'}
    <AnimatedRow {PM} {layer} label="Shape" path="c.shape"><SelectField {PM} get={get('shape', 'rect')} edit={edit('shape', 'Shape')} options={['rect', 'ellipse', 'polygon', 'star', 'line']} label="Shape" /></AnimatedRow>
  {/if}
  <AnimatedRow {PM} {layer} label="Width" path="c.w"><NumField {PM} get={get('w', 0)} edit={edit('w', 'Width')} label="Width" step={1} min={1} unit="px" /></AnimatedRow>
  <AnimatedRow {PM} {layer} label="Height" path="c.h"><NumField {PM} get={get('h', 0)} edit={edit('h', 'Height')} label="Height" step={1} min={1} unit="px" /></AnimatedRow>
  <AnimatedRow {PM} {layer} label="Corner radius" path="c.radius"><NumField {PM} get={get('radius', 0)} edit={edit('radius', 'Corner radius')} label="Corner radius" step={1} min={0} unit="px" /></AnimatedRow>
  {#if layer.type === 'shape'}
    <AnimatedRow {PM} {layer} label="Stroke" path="c.stroke"><NumField {PM} get={get('stroke', 0)} edit={edit('stroke', 'Stroke')} label="Stroke" step={0.5} min={0} unit="px" /></AnimatedRow>
    <AnimatedRow {PM} {layer} label="Stroke color" path="c.strokeColor"><ColorField {PM} get={get('strokeColor', '#FFFFFF')} edit={edit('strokeColor', 'Stroke')} label="Stroke" /></AnimatedRow>
    {#if shape === 'polygon' || shape === 'star'}
      <AnimatedRow {PM} {layer} label="Points" path="c.points"><NumField {PM} get={get('points', 5)} edit={edit('points', 'Points')} label="Points" step={1} min={3} max={24} /></AnimatedRow>
    {/if}
  {/if}
{:else if layer.type === 'image' || layer.type === 'video'}
  <AnimatedRow {PM} {layer} label="Source"><SelectField {PM} get={get('asset', null)} edit={edit('asset', 'Source')} options={mediaOptions(layer.type)} label="Source" /></AnimatedRow>
  <AnimatedRow {PM} {layer} label="Fit" path="c.fit"><SelectField {PM} get={get('fit', 'cover')} edit={edit('fit', 'Fit')} options={['cover', 'contain', 'stretch']} label="Fit" /></AnimatedRow>
  <AnimatedRow {PM} {layer} label="Width" path="c.w"><NumField {PM} get={get('w', 0)} edit={edit('w', 'Width')} label="Width" step={1} min={1} unit="px" /></AnimatedRow>
  <AnimatedRow {PM} {layer} label="Height" path="c.h"><NumField {PM} get={get('h', 0)} edit={edit('h', 'Height')} label="Height" step={1} min={1} unit="px" /></AnimatedRow>
  {#if layer.type === 'video'}
    <AnimatedRow {PM} {layer} label="Trim start" path="c.trim"><NumField {PM} get={get('trim', 0)} edit={edit('trim', 'Trim start')} label="Trim start" step={0.05} precision={2} unit="s" /></AnimatedRow>
    <AnimatedRow {PM} {layer} label="Speed" path="c.speed"><NumField {PM} get={get('speed', 1)} edit={edit('speed', 'Speed')} label="Speed" step={0.05} precision={2} min={0.05} /></AnimatedRow>
  {/if}
{:else if layer.type === 'audio'}
  <AnimatedRow {PM} {layer} label="Trim start" path="c.trim"><NumField {PM} get={get('trim', 0)} edit={edit('trim', 'Trim start')} label="Trim start" step={0.05} precision={2} min={0} unit="s" /></AnimatedRow>
  <AnimatedRow {PM} {layer} label="Gain" path="c.gain"><NumField {PM} get={get('gain', 1)} edit={edit('gain', 'Gain')} label="Gain" step={0.05} precision={2} min={0} max={4} /></AnimatedRow>
  <AnimatedRow {PM} {layer} label="Fade in" path="c.fadeIn"><NumField {PM} get={get('fadeIn', 0)} edit={edit('fadeIn', 'Fade in')} label="Fade in" step={0.05} precision={2} min={0} max={layer.dur} unit="s" /></AnimatedRow>
  <AnimatedRow {PM} {layer} label="Fade out" path="c.fadeOut"><NumField {PM} get={get('fadeOut', 0)} edit={edit('fadeOut', 'Fade out')} label="Fade out" step={0.05} precision={2} min={0} max={layer.dur} unit="s" /></AnimatedRow>
{:else if layer.type === 'shader'}
  <button
    type="button"
    class="chip"
    style="width:100%;justify-content:center;height:30px"
    onclick={() => PM.openShaderEditor?.(layer)}
  >Edit shader source</button>
  {#if shaderError}
    <div role="status" style="font-size:var(--fs-xs);color:var(--red);padding:6px 4px;white-space:pre-wrap;max-height:90px;overflow:auto">{shaderError}</div>
  {/if}
  <AnimatedRow {PM} {layer} label="Width" path="c.w"><NumField {PM} get={get('w', 0)} edit={edit('w', 'Width')} label="Width" step={1} min={1} unit="px" /></AnimatedRow>
  <AnimatedRow {PM} {layer} label="Height" path="c.h"><NumField {PM} get={get('h', 0)} edit={edit('h', 'Height')} label="Height" step={1} min={1} unit="px" /></AnimatedRow>
{:else if layer.type === 'extension'}
  <AnimatedRow {PM} {layer} label="Width" path="c.w"><NumField {PM} get={get('w', 0)} edit={edit('w', 'Width')} label="Width" step={1} min={1} unit="px" /></AnimatedRow>
  <AnimatedRow {PM} {layer} label="Height" path="c.h"><NumField {PM} get={get('h', 0)} edit={edit('h', 'Height')} label="Height" step={1} min={1} unit="px" /></AnimatedRow>
{/if}
