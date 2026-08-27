<script lang="ts">
  import { onDestroy } from 'svelte';
  import { inspectorContext, type EditBinding, type SelectOption } from './context';

  const { api, doc } = inspectorContext();
  const { ColorField, FontField, NumField, Row, Section, SelectField } = api.ui.controls;
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

  const content = $derived((doc.tick.values, doc.proj, layer.d ?? {}));
  const shape = $derived((doc.tick.values, doc.proj, content.shape));
  const assets = $derived((doc.tick.assets, doc.tick.structure, doc.proj, Object.values(PM.proj?.assets ?? {}) as any[]));
  const shaderError = $derived((doc.tick.values, doc.proj,
    layer.type === 'shader' ? PM.GL?.compileError?.(PM.UIState?.getShaderMeta?.(layer)?.shaderKey) : ''));
  let editingText = false;

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

  const audioOptions = $derived<SelectOption[]>([
    { v: null, label: 'None' },
    ...assets
      .filter((asset) => asset?.kind === 'audio')
      .map((asset) => ({ v: asset.id, label: String(asset.name) }))
  ]);
</script>

<Section title="Content" />

{#if layer.type === 'text'}
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
  <Row label="Font">
    {#key fontsVersion}
      <FontField
        {PM}
        get={get('font', '')}
        edit={edit('font', 'Font')}
        label="Font"
        weight={() => Number(content.weight) || 400}
      />
    {/key}
  </Row>
  {@const weights = [...new Set([Number(content.weight) || 400, 100, 200, 300, 400, 500, 600, 700, 800, 900])].sort((a, b) => a - b)}
  <Row label="Weight">
    <SelectField
      {PM}
      get={get('weight', 400)}
      edit={edit('weight', 'Weight')}
      options={weights.map((value) => ({ v: value, label: String(value) }))}
      label="Weight"
      onChange={(value: unknown) => PM.Fonts?.ensure?.(content.font, Number(value) || 400)}
    />
  </Row>
  <Row label="Size"><NumField {PM} get={get('size', 0)} edit={edit('size', 'Size')} label="Size" step={1} min={4} unit="px" /></Row>
  <Row label="Tracking"><NumField {PM} get={get('tracking', 0)} edit={edit('tracking', 'Tracking')} label="Tracking" step={0.5} unit="px" /></Row>
  <Row label="Leading"><NumField {PM} get={get('leading', 0)} edit={edit('leading', 'Leading')} label="Leading" step={0.02} precision={2} /></Row>
  <Row label="Align"><SelectField {PM} get={get('align', 'center')} edit={edit('align', 'Align')} options={['left', 'center', 'right']} label="Align" /></Row>
  <Row label="Color"><ColorField {PM} get={get('color', '#F2F2F2')} edit={edit('color', 'Text color')} label="Text color" /></Row>
{:else if layer.type === 'solid' || layer.type === 'shape'}
  <Row label="Fill"><ColorField {PM} get={get('color', '#808080')} edit={edit('color', 'Fill')} label="Fill" /></Row>
  {#if layer.type === 'shape'}
    <Row label="Shape"><SelectField {PM} get={get('shape', 'rect')} edit={edit('shape', 'Shape')} options={['rect', 'ellipse', 'polygon', 'star', 'line']} label="Shape" /></Row>
  {/if}
  <Row label="Width"><NumField {PM} get={get('w', 0)} edit={edit('w', 'Width')} label="Width" step={1} min={1} unit="px" /></Row>
  <Row label="Height"><NumField {PM} get={get('h', 0)} edit={edit('h', 'Height')} label="Height" step={1} min={1} unit="px" /></Row>
  <Row label="Corner radius"><NumField {PM} get={get('radius', 0)} edit={edit('radius', 'Corner radius')} label="Corner radius" step={1} min={0} unit="px" /></Row>
  {#if layer.type === 'shape'}
    <Row label="Stroke"><NumField {PM} get={get('stroke', 0)} edit={edit('stroke', 'Stroke')} label="Stroke" step={0.5} min={0} unit="px" /></Row>
    <Row label="Stroke color"><ColorField {PM} get={get('strokeColor', '#FFFFFF')} edit={edit('strokeColor', 'Stroke')} label="Stroke" /></Row>
    {#if shape === 'polygon' || shape === 'star'}
      <Row label="Points"><NumField {PM} get={get('points', 5)} edit={edit('points', 'Points')} label="Points" step={1} min={3} max={24} /></Row>
    {/if}
  {/if}
{:else if layer.type === 'image' || layer.type === 'video'}
  <Row label="Source"><SelectField {PM} get={get('asset', null)} edit={edit('asset', 'Source')} options={mediaOptions(layer.type)} label="Source" /></Row>
  <Row label="Fit"><SelectField {PM} get={get('fit', 'cover')} edit={edit('fit', 'Fit')} options={['cover', 'contain', 'stretch']} label="Fit" /></Row>
  <Row label="Width"><NumField {PM} get={get('w', 0)} edit={edit('w', 'Width')} label="Width" step={1} min={1} unit="px" /></Row>
  <Row label="Height"><NumField {PM} get={get('h', 0)} edit={edit('h', 'Height')} label="Height" step={1} min={1} unit="px" /></Row>
  {#if layer.type === 'video'}
    <Row label="Trim start"><NumField {PM} get={get('trim', 0)} edit={edit('trim', 'Trim start')} label="Trim start" step={0.05} precision={2} unit="s" /></Row>
    <Row label="Speed"><NumField {PM} get={get('speed', 1)} edit={edit('speed', 'Speed')} label="Speed" step={0.05} precision={2} min={0.05} /></Row>
  {/if}
{:else if layer.type === 'audio'}
  <Row label="Source"><SelectField {PM} get={get('asset', null)} edit={edit('asset', 'Audio source')} options={audioOptions} label="Audio source" /></Row>
  <Row label="Trim start"><NumField {PM} get={get('trim', 0)} edit={edit('trim', 'Trim start')} label="Trim start" step={0.05} precision={2} min={0} unit="s" /></Row>
  <Row label="Gain"><NumField {PM} get={get('gain', 1)} edit={edit('gain', 'Gain')} label="Gain" step={0.05} precision={2} min={0} max={4} /></Row>
  <Row label="Fade in"><NumField {PM} get={get('fadeIn', 0)} edit={edit('fadeIn', 'Fade in')} label="Fade in" step={0.05} precision={2} min={0} max={layer.dur} unit="s" /></Row>
  <Row label="Fade out"><NumField {PM} get={get('fadeOut', 0)} edit={edit('fadeOut', 'Fade out')} label="Fade out" step={0.05} precision={2} min={0} max={layer.dur} unit="s" /></Row>
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
  <Row label="Width"><NumField {PM} get={get('w', 0)} edit={edit('w', 'Width')} label="Width" step={1} min={1} unit="px" /></Row>
  <Row label="Height"><NumField {PM} get={get('h', 0)} edit={edit('h', 'Height')} label="Height" step={1} min={1} unit="px" /></Row>
{/if}
