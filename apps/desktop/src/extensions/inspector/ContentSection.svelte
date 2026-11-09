<script lang="ts">
  import AnimatedRow from './AnimatedRow.svelte';
  import { onDestroy } from 'svelte';
  import { inspectorContext, type EditBinding, type SelectOption } from './context';
  import TypeSettings from './TypeSettings.svelte';
  import TextAlignment from './TextAlignment.svelte';
  import PaintRows from './PaintRows.svelte';
  import StrokeRows from './StrokeRows.svelte';
  import ArcRows from './ArcRows.svelte';
  import ImageMappingRows from './ImageMappingRows.svelte';
  import TextRunList from './TextRunList.svelte';
  import Choice from './Choice.svelte';
  import { editableSelection } from './multi-edit';
  import { propertyCommand, rawAt, RUN_FIELDS, VERTICAL_ALIGN, DECORATIONS, TEXT_CASES } from './figma-data';
  import { precompCarriesAudio } from './audio';
  import { axisContentKey, type FontStyle } from 'powermove';

  const { api, doc, sel, transport, mixed, edit: inspectorEdit } = inspectorContext();
  const { ColorField, FontField, NumField, Section, SelectField, SliderField, ToggleField } = api.ui.controls;
  const { contentBinding } = api.ui.controls.binding;
  const weightLabels: Record<number, string> = {
    100: 'Thin',
    200: 'Extra Light',
    300: 'Light',
    400: 'Regular',
    500: 'Medium',
    600: 'Semi Bold',
    700: 'Bold',
    800: 'Extra Bold',
    900: 'Black',
  };

  let { layer, fontsVersion = 0 }: { layer: any; fontsVersion?: number } = $props();

  const content = $derived((doc.tick.values, doc.proj, transport.time, api.anim.resolveContent(layer, transport.time) as Record<string, any>));
  const modelLayer = $derived((doc.tick.structure, doc.proj, layer.type==='extension' && api.layers.get(String(layer.d?.definition || ''))?.renderer.kind==='layer3d'));
  const shape = $derived((doc.tick.values, doc.proj, content.shape));
  const assets = $derived((doc.tick.assets, doc.tick.structure, doc.proj, Object.values(api.project.get()?.assets ?? {}) as any[]));
  const precompAudible = $derived((doc.tick.structure, doc.tick.values, doc.proj,
    layer.type === 'precomp' && precompCarriesAudio(layer.d?.comp, api.project.get()?.comps as any)));
  const shaderMeta = $derived((doc.tick.values, doc.proj, api.uiState.getShaderMeta(layer)));
  const shaderError = $derived((doc.tick.values, doc.proj,
    typeof shaderMeta?.shaderKey === 'string' ? api.render.gl.compileError(shaderMeta.shaderKey) ?? '' : ''));
  let editingText = false;
  let hasVariableWeight = $state(false);
  let fontStyles = $state<FontStyle[]>([]);
  const IMPORT_SOURCE = '__powermove_import_source__';

  const get = (key: string, fallback?: unknown) => () => content[key] == null ? fallback : content[key];
  const edit = (key: string, label: string): EditBinding =>
    contentBinding(layer.id, key, { label, origin: 'inspector' });

  /* Styled (rich) text keeps per-range styles in `runs`, which override the
     layer's base style. Its whole-text rows show the shared value ("Mixed"
     when ranges differ) and an edit sets the base and every range at once. */
  const runs = $derived((layer.type === 'text' && content.richText && Array.isArray(content.runs) ? content.runs : []) as Array<Record<string, any>>);
  const rich = $derived(runs.length > 0);
  const runValue = (run: Record<string, any>, key: string) => {
    if (key === 'leading') {
      const size = Number(run.size ?? content.size) || 1;
      return Math.round(((Number(run.leading) || (Number(content.leading) || 1.2) * size) / size) * 100) / 100;
    }
    return run[key] ?? content[key];
  };
  const shared = (key: string, fallback?: unknown) => () => rich ? runValue(runs[0]!, key) ?? fallback : (content[key] == null ? fallback : content[key]);
  const runsDiffer = (key: string) => rich && runs.some((run) => JSON.stringify(runValue(run, key)) !== JSON.stringify(runValue(runs[0]!, key)));
  const richMixed = (key: string | string[]) => (binding: EditBinding, value: unknown) =>
    [key].flat().some(runsDiffer) || mixed(binding, value);
  const runCommands = (target: any, patch: Record<string, unknown>) => {
    const now = api.anim.resolveContent(target, transport.time) as Record<string, any>;
    if (!now.richText) return [];
    return Object.entries(patch).flatMap(([key, value]) => (RUN_FIELDS as readonly string[]).includes(key)
      ? ((now.runs ?? []) as Array<Record<string, any>>).flatMap((run, index) => rawAt(target, `runs.${index}.${key}`) === undefined ? []
        : [propertyCommand(target.id, `c.runs.${index}.${key}`, key === 'leading' ? Math.round(Number(value) * (Number(run.size ?? now.size) || 0) * 100) / 100 : value, transport.time)])
      : []);
  };
  /* Wrap a base-field binding so it also writes every range, on each selected
     text layer from its own ranges. */
  const richEdit = (binding: EditBinding): EditBinding => !rich || binding.mode !== 'command' ? binding : {
    ...binding,
    command: (value) => {
      const base = ([] as any[]).concat(typeof binding.command === 'function' ? binding.command(value) : binding.command);
      const texts = editableSelection(api, api.model.layer(layer.id) ?? layer).filter((target: any) => target.type === 'text');
      return texts.flatMap((target: any) => base.flatMap((command) => [
        { ...command, target: target.id },
        ...(command.type === 'set_content' ? runCommands(target, command.patch ?? {}) : [])
      ]));
    }
  };
  /* One range keys its own channel; several ranges key per style, below. */
  const richPath = (key: string) => !rich ? `c.${key}` : runs.length === 1 && rawAt(layer, `runs.0.${key}`) !== undefined ? `c.runs.0.${key}` : undefined;
  const textWeight = $derived(Number(rich ? runValue(runs[0]!, 'weight') : content.weight) || 400);
  const textItalic = $derived(!!(rich ? runValue(runs[0]!, 'italic') : content.italic));
  const multiline = $derived(String(content.text ?? '').includes('\n'));
  const truncating = $derived(String(content.truncation ?? 'DISABLED') === 'ENDING');
  const advancedPaths = $derived(layer.type === 'shape' && !!layer.d.paths?.length && !!(content.fillGradient || content.strokeGradient || content.strokeStyle || content.compound));

  const CORNERS = [['radiusTL', 'Top left'], ['radiusTR', 'Top right'], ['radiusBL', 'Bottom left'], ['radiusBR', 'Bottom right']] as const;

  /* Turning independent corners on seeds every corner from the current
     uniform radius so the shape does not change until a corner is edited. */
  const independentPatch = (on: boolean) => ({
    type: 'set_content' as const,
    target: layer.id,
    patch: on
      ? { independentCorners: true, ...Object.fromEntries(CORNERS.map(([key]) => [key, Number(content.radius) || 0])) }
      : { independentCorners: false }
  });

  const CORNER_GLYPHS = {
    radiusTL: 'M3 13V7a4 4 0 0 1 4-4h6',
    radiusTR: 'M3 3h6a4 4 0 0 1 4 4v6',
    radiusBR: 'M13 3v6a4 4 0 0 1-4 4H3',
    radiusBL: 'M3 3v6a4 4 0 0 0 4 4h6'
  } as const;
  const ALL_CORNERS_GLYPH = 'M2.5 5.5V4a1.5 1.5 0 0 1 1.5-1.5h1.5M10.5 2.5H12A1.5 1.5 0 0 1 13.5 4v1.5M13.5 10.5V12a1.5 1.5 0 0 1-1.5 1.5h-1.5M5.5 13.5H4A1.5 1.5 0 0 1 2.5 12v-1.5';
  /* Smoothing is Figma's corner smoothing: 0% is a circular arc, about 60%
     matches Apple's continuous (squircle) corners, 100% is the smoothest. */
  const IOS_SMOOTHING = 60;
  const SMOOTHING_HINT = 'Corner smoothing blends each rounded corner into its straight edges, like the continuous corners on iOS and in Figma. 0% is a circular corner; about 60% matches iOS. Pills keep perfectly round ends at any smoothing.';

  const cornerSummary = $derived.by(() => {
    const values = CORNERS.map(([key]) => Number(content[key] ?? content.radius) || 0);
    return values.every((v) => v === values[0]) ? `${values[0]}px` : 'Mixed';
  });

  /* A narrow inspector drops the glyph inside the radius field. */
  const CORNER_COMPACT_WIDTH = 120;
  let cornerCompact = $state(false);
  function measureCorners(node: HTMLElement) {
    const observer = new ResizeObserver(() => { cornerCompact = node.clientWidth < CORNER_COMPACT_WIDTH; });
    observer.observe(node);
    return { destroy: () => observer.disconnect() };
  }

  function beginText(): void {
    if (editingText) return;
    inspectorEdit.begin('Edit text', { origin: 'inspector' });
    editingText = true;
  }

  function inputText(event: Event): void {
    if (!editingText) beginText();
    inspectorEdit.dispatch({
      type: 'set_content',
      target: layer.id,
      patch: { text: (event.currentTarget as HTMLTextAreaElement).value }
    });
    api.transport.invalidate?.('render');
  }

  function commitText(): void {
    if (!editingText) return;
    editingText = false;
    inspectorEdit.commit('Edit text');
  }

  onDestroy(commitText);

  /* A static family lists the faces it actually has, by their own names. The
     selected face is the one the renderer draws: like CSS font matching, the
     same slope wins first, then the nearest weight. */
  const styleKey = (style: { weight: number; italic: boolean }) => `${style.weight}${style.italic ? ' italic' : ''}`;
  const currentStyle = $derived.by(() => {
    const weight = textWeight, italic = textItalic;
    const slope = fontStyles.filter((style) => style.italic === italic);
    return (slope.length ? slope : fontStyles).reduce<FontStyle | null>((best, style) =>
      !best || Math.abs(style.weight - weight) < Math.abs(best.weight - weight) ? style : best, null);
  });
  const styleEdit: EditBinding = {
    mode: 'command',
    label: 'Style',
    origin: 'inspector',
    command: (value) => {
      const style = fontStyles.find((candidate) => styleKey(candidate) === value);
      return style ? { type: 'set_content', target: layer.id, patch: { weight: style.weight, italic: style.italic } } : [];
    }
  };

  function mediaOptions(kind: 'image' | 'video'): SelectOption[] {
    return assets
      .filter((asset) => asset?.kind === kind)
      .map((asset) => ({ v: asset.id, label: String(asset.name) }))
      .concat([{ v: IMPORT_SOURCE, label: 'Import file…' }])
      .concat([{ v: null, label: 'none' }]);
  }

  async function importSource(kind: 'image' | 'video'): Promise<void> {
    try {
      const files = await api.assets.pick({ accept: kind === 'image' ? 'image/*' : 'video/*' });
      const file = files[0];
      if (!file) return;
      if (api.media.assets.kind(file) !== kind) throw new Error(`Choose a ${kind} file for this source`);
      const asset = await api.assets.import(file);
      inspectorEdit.apply({
        type: 'set_content',
        target: layer.id,
        patch: { asset: asset.id }
      }, { label: 'Import source', origin: 'inspector' });
      api.transport.invalidate();
    } catch (error) {
      api.ui.toast(error instanceof Error ? error.message : 'Could not import this file', { kind: 'alert', error: true });
    }
  }

  function selectSource(kind: 'image' | 'video', value: unknown): boolean | void {
    if (value !== IMPORT_SOURCE) return;
    void importSource(kind);
    return false;
  }


</script>

{#if !modelLayer && !(layer.type === 'shape' && layer.d.paths?.length && !advancedPaths)}<Section {api} title="Content" />{/if}

<div class="content-rows">
{#if layer.type === 'text'}
  <AnimatedRow {layer} label="Text" path="c.text">
  <textarea
    data-inspector-text-layer={layer.id}
    aria-label="Text"
    value={String(content.text ?? '')}
    class="text-content"
    rows="1"
    onfocus={beginText}
    oninput={inputText}
    onblur={commitText}
    onkeydown={(event) => event.stopPropagation()}
  ></textarea>
  </AnimatedRow>
  <AnimatedRow {layer} label="Font" path={richPath('font')}>
    {#key fontsVersion}
      <FontField
        {api}
        mixed={richMixed('font')}

        get={shared('font', '')}
        edit={richEdit(edit('font', 'Font'))}
        label="Font"
        weight={() => textWeight}
        styleName={() => runsDiffer('weight') || runsDiffer('italic') ? '' : currentStyle?.name ?? weightLabels[textWeight] ?? ''}
      />
    {/key}
  </AnimatedRow>
  <AnimatedRow {layer} label="Size" path={richPath('size')}><NumField {api} mixed={richMixed('size')} get={shared('size', 0)} edit={richEdit(edit('size', 'Size'))} label="Size" step={1} min={4} unit="px" /></AnimatedRow>
  <AnimatedRow {layer} label="Color" path={richPath('color')}><ColorField {api} mixed={richMixed('color')} get={shared('color', '#F2F2F2')} edit={richEdit(edit('color', 'Text color'))} label="Text color" /></AnimatedRow>
  <!-- Text, Font, Size and Color first, then the rest of the type settings. -->
  {#if !hasVariableWeight && content[axisContentKey('wght')] == null && fontStyles.length}
    <AnimatedRow {layer} label="Style" path={richPath('weight')}>
      <SelectField
        {api}
        mixed={richMixed(['weight', 'italic'])}

        get={() => currentStyle ? styleKey(currentStyle) : null}
        edit={richEdit(styleEdit)}
        options={fontStyles.map((style) => ({ v: styleKey(style), label: style.name }))}
        label="Style"
        onChange={(value: unknown) => {
          const style = fontStyles.find((candidate) => styleKey(candidate) === value);
          if (style) api.media.fonts?.ensure?.(content.font, style.weight);
        }}
      />
    </AnimatedRow>
  {:else if !hasVariableWeight && content[axisContentKey('wght')] == null}
    {@const weights = [...new Set([textWeight, 100, 200, 300, 400, 500, 600, 700, 800, 900])].sort((a, b) => a - b)}
    <AnimatedRow {layer} label="Weight" path={richPath('weight')}>
      <SelectField
        {api}
        mixed={richMixed('weight')}

        get={shared('weight', 400)}
        edit={richEdit(edit('weight', 'Weight'))}
        options={weights.map((value) => ({ v: value, label: weightLabels[value] ?? String(value) }))}
        label="Weight"
        onChange={(value: unknown) => api.media.fonts?.ensure?.(content.font, Number(value) || 400)}
      />
    </AnimatedRow>
  {/if}
  <AnimatedRow {layer} label="Tracking" path={richPath('tracking')}><NumField {api} mixed={richMixed('tracking')} get={shared('tracking', 0)} edit={richEdit(edit('tracking', 'Tracking'))} label="Tracking" step={0.5} unit="px" /></AnimatedRow>
  <AnimatedRow {layer} label="Leading" path={rich ? undefined : 'c.leading'}><NumField {api} mixed={richMixed('leading')} get={shared('leading', 0)} edit={richEdit(edit('leading', 'Leading'))} label="Leading" step={0.02} precision={2} /></AnimatedRow>
  <AnimatedRow {layer} label="Align" path="c.align"><TextAlignment {layer} value={String(content.align ?? 'center')} /></AnimatedRow>
  {#if rich}
    <AnimatedRow {layer} label="Decoration" path={richPath('decoration')}><SelectField {api} mixed={richMixed('decoration')} get={shared('decoration', 'NONE')} edit={richEdit(edit('decoration', 'Decoration'))} options={DECORATIONS} label="Decoration" /></AnimatedRow>
    <AnimatedRow {layer} label="Case" path={richPath('textCase')}><SelectField {api} mixed={richMixed('textCase')} get={shared('textCase', 'ORIGINAL')} edit={richEdit(edit('textCase', 'Case'))} options={TEXT_CASES} label="Case" /></AnimatedRow>
    {#if Number(content.boxHeight) > 0}
      <AnimatedRow {layer} label="Vertical" path="c.verticalAlign"><Choice value={String(content.verticalAlign ?? 'top')} options={VERTICAL_ALIGN} label="Vertical alignment" edit={edit('verticalAlign', 'Vertical alignment')} /></AnimatedRow>
    {/if}
    {#if multiline || runs.some((run) => Number(runValue(run, 'paragraphSpacing')) || Number(runValue(run, 'paragraphIndent')))}
      <AnimatedRow {layer} label="Spacing" path={richPath('paragraphSpacing')}><NumField {api} mixed={richMixed('paragraphSpacing')} get={shared('paragraphSpacing', 0)} edit={richEdit(edit('paragraphSpacing', 'Paragraph spacing'))} label="Paragraph spacing" step={1} min={0} unit="px" /></AnimatedRow>
      <AnimatedRow {layer} label="Indent" path={richPath('paragraphIndent')}><NumField {api} mixed={richMixed('paragraphIndent')} get={shared('paragraphIndent', 0)} edit={richEdit(edit('paragraphIndent', 'Paragraph indent'))} label="Paragraph indent" step={1} unit="px" /></AnimatedRow>
    {/if}
    <AnimatedRow {layer} label="Truncate" path="c.truncation"><ToggleField {api} {mixed} get={() => truncating} edit={{ mode: 'command', label: 'Truncate text', origin: 'inspector', command: (value: unknown) => ({ type: 'set_content', target: layer.id, patch: { truncation: value === true || value === 'true' ? 'ENDING' : 'DISABLED' } }) }} label="Truncate text" /></AnimatedRow>
    {#if truncating || Number(content.maxLines) > 0}
      <AnimatedRow {layer} label="Max lines" path="c.maxLines"><NumField {api} {mixed} get={() => Number(content.maxLines) || 0} edit={edit('maxLines', 'Max lines')} label="Max lines" step={1} min={0} precision={0} /></AnimatedRow>
    {/if}
  {/if}
  {#if layer.d.textStroke !== undefined}
    <AnimatedRow {layer} label="Stroke" path="c.textStroke"><NumField {api} {mixed} get={get('textStroke', 0)} edit={edit('textStroke', 'Text stroke')} label="Text stroke" step={0.5} min={0} unit="px" /></AnimatedRow>
    {#if Number(content.textStroke) > 0}
      <AnimatedRow {layer} label="Stroke color" path="c.textStrokeColor"><ColorField {api} {mixed} get={get('textStrokeColor', '#000000')} edit={edit('textStrokeColor', 'Text stroke color')} label="Text stroke color" /></AnimatedRow>
    {/if}
  {/if}
  <TypeSettings {layer} family={String(content.font ?? '')} onVariableWeight={(value) => { hasVariableWeight = value; }} onStyles={(styles) => { fontStyles = styles; }} />
  <!-- Ranges belong to one layer's words; a selection edits whole texts above. -->
  {#if runs.length > 1 && sel.layers.length < 2}<TextRunList {layer} {content} {fontsVersion} />{/if}
{:else if (layer.type === 'solid' || layer.type === 'shape' || layer.type === 'null') && !layer.d.paths?.length}
  {#if layer.type === 'shape' && shape !== 'line'}
    <PaintRows {layer} {content} kind="fill" label="Fill" />
  {:else}
    <AnimatedRow {layer} label="Fill" path="c.color"><ColorField {api} {mixed} get={get('color', '#808080')} edit={edit('color', 'Fill')} label="Fill" /></AnimatedRow>
  {/if}
  {#if layer.type === 'shape'}
    <AnimatedRow {layer} label="Shape" path="c.shape"><SelectField {api} {mixed} get={get('shape', 'rect')} edit={edit('shape', 'Shape')} options={['rect', 'ellipse', 'polygon', 'star', 'line']} label="Shape" /></AnimatedRow>
  {/if}
  <AnimatedRow {layer} label="Width" path="c.w"><NumField {api} {mixed} get={get('w', 0)} edit={edit('w', 'Width')} label="Width" step={1} min={1} unit="px" /></AnimatedRow>
  <AnimatedRow {layer} label="Height" path="c.h"><NumField {api} {mixed} get={get('h', 0)} edit={edit('h', 'Height')} label="Height" step={1} min={1} unit="px" /></AnimatedRow>
  {#if layer.type === 'shape' && shape === 'rect'}
    <AnimatedRow {layer} label="Corner radius" path="c.radius">
      <div class="corner-inline" class:compact={cornerCompact} use:measureCorners>
        <div class="corner-well" title="Corner radius">
          {#if !cornerCompact}<svg class="corner-glyph" aria-hidden="true" viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round"><path d={ALL_CORNERS_GLYPH}/></svg>{/if}
          {#if content.independentCorners}
            <span class="corner-readout" class:compact={cornerCompact}>{cornerSummary}</span>
          {:else}
            <NumField {api} {mixed} get={get('radius', 0)} edit={edit('radius', 'Corner radius')} label="Corner radius" step={1} min={0} unit="px" />
          {/if}
        </div>
        <button type="button" class="corner-btn" class:on={!!content.independentCorners} title="Independent corners" aria-label="Independent corners" aria-pressed={!!content.independentCorners}
          onclick={() => inspectorEdit.apply(independentPatch(!content.independentCorners), { label: 'Independent corners', origin: 'inspector' })}>
          <svg viewBox="0 0 16 16" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round"><path d={ALL_CORNERS_GLYPH}/></svg>
        </button>
      </div>
    </AnimatedRow>
    {#if content.independentCorners}
      <div class="corner-grid">
        {#each CORNERS as [key, name] (key)}
          <div class="corner-well" title={name}>
            <svg class="corner-glyph" aria-hidden="true" viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round"><path d={CORNER_GLYPHS[key]} /></svg>
            <NumField {api} {mixed} get={get(key, content.radius ?? 0)} edit={edit(key, name)} label={name} step={1} min={0} />
          </div>
        {/each}
      </div>
    {/if}
    <AnimatedRow {layer} label="Smoothing" path="c.smoothing">
      <div class="smoothing" title={SMOOTHING_HINT} data-smoothing-control>
        <SliderField {api} {mixed} get={get('smoothing', 0)} edit={edit('smoothing', 'Corner smoothing')} label="Smoothing" min={0} max={100} step={1} unit="%" />
        <i class="smoothing-ios" class:hit={Number(content.smoothing) === IOS_SMOOTHING} style={`left:${IOS_SMOOTHING}%`} aria-hidden="true"></i>
      </div>
    </AnimatedRow>
  {:else if layer.type === 'shape' && shape === 'ellipse'}
    <ArcRows {layer} {content} />
  {:else}
    <AnimatedRow {layer} label="Corner radius" path="c.radius"><NumField {api} {mixed} get={get('radius', 0)} edit={edit('radius', 'Corner radius')} label="Corner radius" step={1} min={0} unit="px" /></AnimatedRow>
  {/if}
  {#if layer.type === 'shape'}
    <StrokeRows {layer} {content} />
    {#if shape === 'polygon' || shape === 'star'}
      <AnimatedRow {layer} label="Points" path="c.points"><NumField {api} {mixed} get={get('points', 5)} edit={edit('points', 'Points')} label="Points" step={1} min={3} max={24} /></AnimatedRow>
    {/if}
  {/if}
{:else if advancedPaths}
  <!-- Imported vector paths paint as one shape: its fill, stroke and stroke style. -->
  <PaintRows {layer} {content} kind="fill" label="Fill" />
  <StrokeRows {layer} {content} />
{:else if layer.type === 'image' || layer.type === 'video'}
  <AnimatedRow {layer} label="Source"><SelectField {api} {mixed} get={get('asset', null)} edit={edit('asset', 'Source')} options={mediaOptions(layer.type)} label="Source" onChange={(value: unknown) => selectSource(layer.type, value)} /></AnimatedRow>
  {#if layer.type === 'image' && content.imageMapping}
    <!-- A Figma image fill maps by its own mode; Fit does not apply. -->
    <ImageMappingRows {layer} {content} />
  {:else}
    <AnimatedRow {layer} label="Fit" path="c.fit"><SelectField {api} {mixed} get={get('fit', 'cover')} edit={edit('fit', 'Fit')} options={['cover', 'contain', 'stretch']} label="Fit" /></AnimatedRow>
  {/if}
  <AnimatedRow {layer} label="Width" path="c.w"><NumField {api} {mixed} get={get('w', 0)} edit={edit('w', 'Width')} label="Width" step={1} min={1} unit="px" /></AnimatedRow>
  <AnimatedRow {layer} label="Height" path="c.h"><NumField {api} {mixed} get={get('h', 0)} edit={edit('h', 'Height')} label="Height" step={1} min={1} unit="px" /></AnimatedRow>
  {#if layer.type === 'video'}
    <AnimatedRow {layer} label="Trim start" path="c.trim"><NumField {api} {mixed} get={get('trim', 0)} edit={edit('trim', 'Trim start')} label="Trim start" step={0.05} precision={2} unit="s" /></AnimatedRow>
    <AnimatedRow {layer} label="Speed" path="c.speed"><NumField {api} {mixed} get={get('speed', 1)} edit={edit('speed', 'Speed')} label="Speed" step={0.05} precision={2} min={0.05} /></AnimatedRow>
    {#if content.embeddedAudio === true}
      <AnimatedRow {layer} label="Mute audio"><ToggleField {api} {mixed} get={get('audioMuted', false)} edit={edit('audioMuted', 'Mute audio')} label="Mute audio" /></AnimatedRow>
      <AnimatedRow {layer} label="Audio gain" path="c.audioGain"><NumField {api} {mixed} get={get('audioGain', 1)} edit={edit('audioGain', 'Audio gain')} label="Audio gain" step={0.05} precision={2} min={0} max={4} /></AnimatedRow>
    {/if}
  {/if}
{:else if layer.type === 'audio'}
  <AnimatedRow {layer} label="Trim start" path="c.trim"><NumField {api} {mixed} get={get('trim', 0)} edit={edit('trim', 'Trim start')} label="Trim start" step={0.05} precision={2} min={0} unit="s" /></AnimatedRow>
  <AnimatedRow {layer} label="Gain" path="c.gain"><NumField {api} {mixed} get={get('gain', 1)} edit={edit('gain', 'Gain')} label="Gain" step={0.05} precision={2} min={0} max={4} /></AnimatedRow>
  <AnimatedRow {layer} label="Fade in" path="c.fadeIn"><NumField {api} {mixed} get={get('fadeIn', 0)} edit={edit('fadeIn', 'Fade in')} label="Fade in" step={0.05} precision={2} min={0} max={layer.dur} unit="s" /></AnimatedRow>
  <AnimatedRow {layer} label="Fade out" path="c.fadeOut"><NumField {api} {mixed} get={get('fadeOut', 0)} edit={edit('fadeOut', 'Fade out')} label="Fade out" step={0.05} precision={2} min={0} max={layer.dur} unit="s" /></AnimatedRow>
{:else if layer.type === 'precomp'}
  {#if precompAudible}
    <AnimatedRow {layer} label="Mute audio"><ToggleField {api} {mixed} get={get('audioMuted', false)} edit={edit('audioMuted', 'Mute audio')} label="Mute audio" /></AnimatedRow>
    <AnimatedRow {layer} label="Audio gain" path="c.audioGain"><NumField {api} {mixed} get={get('audioGain', 1)} edit={edit('audioGain', 'Audio gain')} label="Audio gain" step={0.05} precision={2} min={0} max={4} /></AnimatedRow>
  {/if}
{:else if layer.type === 'shader'}
  <button
    type="button"
    class="chip"
    style="width:100%;justify-content:center;height:30px"
    onclick={() => api.ui.openShaderEditor?.(layer)}
  >Edit shader source</button>
  {#if shaderError}
    <div role="status" style="font-size:var(--fs-xs);color:var(--red);padding:6px 4px;white-space:pre-wrap;max-height:90px;overflow:auto">{shaderError}</div>
  {/if}
  <AnimatedRow {layer} label="Width" path="c.w"><NumField {api} {mixed} get={get('w', 0)} edit={edit('w', 'Width')} label="Width" step={1} min={1} unit="px" /></AnimatedRow>
  <AnimatedRow {layer} label="Height" path="c.h"><NumField {api} {mixed} get={get('h', 0)} edit={edit('h', 'Height')} label="Height" step={1} min={1} unit="px" /></AnimatedRow>
{:else if layer.type === 'extension' && !modelLayer}
  <AnimatedRow {layer} label="Width" path="c.w"><NumField {api} {mixed} get={get('w', 0)} edit={edit('w', 'Width')} label="Width" step={1} min={1} unit="px" /></AnimatedRow>
  <AnimatedRow {layer} label="Height" path="c.h"><NumField {api} {mixed} get={get('h', 0)} edit={edit('h', 'Height')} label="Height" step={1} min={1} unit="px" /></AnimatedRow>
{/if}
</div>

<style>
  /* The text itself: one line tall, growing with its lines. */
  .text-content {
    width: 100%; min-height: 26px; field-sizing: content; padding: 5px 8px; border: 0; border-radius: 6px;
    background: var(--bg-field); color: var(--tx-value); font: 12px/16px var(--f-ui); resize: none;
  }
  .text-content:focus-visible { outline: 0; box-shadow: inset 0 0 0 1px var(--accent); }
  /* Content rows read as plain label and value, as in the prototype; the
     keyframe diamond sits in the margin, shown under the pointer or while
     the property is animated. */
  .content-rows.content-rows :global(.row.split) { position: relative; }
  .content-rows.content-rows :global(.row.split > .stopwatch) { position: absolute; left: 1px; width: 14px; opacity: 0; }
  .content-rows.content-rows :global(.row.split > .stopwatch.on),
  .content-rows.content-rows :global(.row.split:hover > .stopwatch),
  .content-rows.content-rows :global(.row.split > .stopwatch:focus-visible) { opacity: 1; }
  .content-rows.content-rows :global(.row.split > .k),
  .content-rows.content-rows :global(.row.split:not(:has(> .stopwatch)) > .k) { width: 96px; padding-left: 0; }

  /* Figma-style corner controls: glyphs sit inside the wells, toggles tint
     with the accent when on, and independent corners form a 2x2 grid with
     the toggles in a trailing column. */
  .corner-inline { flex: 1; min-width: 0; height: 100%; display: flex; align-items: center; gap: 2px; }
  .corner-inline.compact .corner-well :global(input.num:not([type=range]):not([type=color]):not([type=file])) { padding-left: 8px; }
  .corner-well { position: relative; flex: 1 1 0; min-width: 0; display: flex; align-items: center; }
  .corner-glyph { position: absolute; left: 7px; color: var(--tx-3); pointer-events: none; }
  .corner-well :global(input.num:not([type=range]):not([type=color]):not([type=file])) { padding-left: 26px; }
  .corner-grid {
    display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 8px;
    margin: 4px calc(-1 * var(--pad)); padding: 0 16px;
    transform-origin: top center; animation: pm-menu-in .18s cubic-bezier(.23, 1, .32, 1) both;
  }
  .corner-readout {
    flex: 1; min-width: 0; height: 28px; display: flex; align-items: center; padding-left: 26px;
    border-radius: var(--r-sm); background: var(--bg-row); color: var(--tx-3); font-size: var(--fs-md);
  }
  .corner-readout.compact { padding-left: 8px; }
  .corner-btn {
    width: 28px; height: 28px; flex: none; padding: 0;
    display: inline-flex; align-items: center; justify-content: center;
    border: 0; border-radius: var(--r-sm); background: transparent; color: var(--tx-2); cursor: pointer;
    transition: background var(--dur-1), color var(--dur-1);
  }
  .corner-btn:hover { background: rgb(var(--ink-rgb) / .08); color: var(--tx); }
  .corner-btn.on { background: color-mix(in srgb, var(--accent) 22%, transparent); color: var(--accent-tx); }
  /* Smoothing: the shared filled slider, with a hairline tick at the iOS value. */
  .smoothing { position: relative; flex: 1; min-width: 0; display: flex; }
  .smoothing-ios {
    position: absolute; bottom: 0; width: 1px; height: 6px; transform: translateX(-50%);
    background: rgb(var(--ink-rgb) / .38); pointer-events: none;
  }
  .smoothing-ios.hit { background: var(--accent-tx); }
  @media (prefers-reduced-motion: reduce) { .corner-grid { animation: none; } }
</style>
