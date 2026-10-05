<script lang="ts">
  import AnimatedRow from './AnimatedRow.svelte';
  import { anchorPicker, mountOverlayOnBody } from 'powermove';
  import { onDestroy } from 'svelte';
  import { inspectorContext, type EditBinding, type SelectOption } from './context';
  import TypeSettings from './TypeSettings.svelte';
  import TextAlignment from './TextAlignment.svelte';
  import { axisContentKey, type FontStyle } from 'powermove';

  const { api, doc, transport, mixed, edit: inspectorEdit } = inspectorContext();
  const { ColorField, FontField, NumField, Section, SelectField, ToggleField } = api.ui.controls;
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

  const CORNERS = [['radiusTL', 'Top left'], ['radiusTR', 'Top right'], ['radiusBL', 'Bottom left'], ['radiusBR', 'Bottom right']] as const;
  const IOS_SMOOTHING = 60;

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
  let smoothingOpen = $state(false);
  /* Centre of a 16px range thumb at `value`%, so the fill and iOS tick line up with it. */
  const thumbAt = (value: number) => `calc(8px + (100% - 16px) * ${value / 100})`;

  let smoothingTrigger: HTMLButtonElement | undefined = $state();

  const cornerSummary = $derived.by(() => {
    const values = CORNERS.map(([key]) => Number(content[key] ?? content.radius) || 0);
    return values.every((v) => v === values[0]) ? `${values[0]}px` : 'Mixed';
  });

  /* A narrow inspector cannot fit the field plus two toggles beside the label:
     below this width the independent-corners toggle moves into the popover. */
  const CORNER_COMPACT_WIDTH = 120;
  let cornerCompact = $state(false);
  function measureCorners(node: HTMLElement) {
    const observer = new ResizeObserver(() => { cornerCompact = node.clientWidth < CORNER_COMPACT_WIDTH; });
    observer.observe(node);
    return { destroy: () => observer.disconnect() };
  }

  /* The popover floats on the body so the inspector's clipped scroller cannot
     cut it off; a press outside it (other than its own toggle) or Escape closes it. */
  function dismissOutside(node: HTMLElement) {
    const close = () => { smoothingOpen = false; };
    const onDown = (event: PointerEvent) => {
      const target = event.target as Node;
      if (!node.contains(target) && !smoothingTrigger?.contains(target)) close();
    };
    const onKey = (event: KeyboardEvent) => { if (event.key === 'Escape') { event.stopPropagation(); close(); smoothingTrigger?.focus(); } };
    document.addEventListener('pointerdown', onDown, true);
    window.addEventListener('keydown', onKey, true);
    return { destroy: () => { document.removeEventListener('pointerdown', onDown, true); window.removeEventListener('keydown', onKey, true); } };
  }
  let draggingSmoothing = false;

  function dragSmoothing(event: Event): void {
    let value = Number((event.currentTarget as HTMLInputElement).value);
    if (Math.abs(value - IOS_SMOOTHING) <= 2) value = IOS_SMOOTHING;
    if (!draggingSmoothing) { inspectorEdit.begin('Corner smoothing', { origin: 'inspector' }); draggingSmoothing = true; }
    inspectorEdit.dispatch({ type: 'set_content', target: layer.id, patch: { smoothing: value } });
    api.transport.invalidate?.('render');
  }

  function endSmoothing(): void {
    if (!draggingSmoothing) return;
    draggingSmoothing = false;
    inspectorEdit.commit('Corner smoothing');
  }

  onDestroy(endSmoothing);

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
    const weight = Number(content.weight) || 400, italic = !!content.italic;
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

{#if !modelLayer && !(layer.type === 'shape' && layer.d.paths?.length)}<Section {api} title="Content" />{/if}

{#if layer.type === 'text'}
  <AnimatedRow {layer} label="Text" path="c.text">
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
  <AnimatedRow {layer} label="Font" path="c.font">
    {#key fontsVersion}
      <FontField
        {api}
        {mixed}

        get={get('font', '')}
        edit={edit('font', 'Font')}
        label="Font"
        weight={() => Number(content.weight) || 400}
      />
    {/key}
  </AnimatedRow>
  {#if !hasVariableWeight && content[axisContentKey('wght')] == null && fontStyles.length}
    <AnimatedRow {layer} label="Style" path="c.weight">
      <SelectField
        {api}
        {mixed}

        get={() => currentStyle ? styleKey(currentStyle) : null}
        edit={styleEdit}
        options={fontStyles.map((style) => ({ v: styleKey(style), label: style.name }))}
        label="Style"
        onChange={(value: unknown) => {
          const style = fontStyles.find((candidate) => styleKey(candidate) === value);
          if (style) api.media.fonts?.ensure?.(content.font, style.weight);
        }}
      />
    </AnimatedRow>
  {:else if !hasVariableWeight && content[axisContentKey('wght')] == null}
    {@const weights = [...new Set([Number(content.weight) || 400, 100, 200, 300, 400, 500, 600, 700, 800, 900])].sort((a, b) => a - b)}
    <AnimatedRow {layer} label="Weight" path="c.weight">
      <SelectField
        {api}
        {mixed}

        get={get('weight', 400)}
        edit={edit('weight', 'Weight')}
        options={weights.map((value) => ({ v: value, label: weightLabels[value] ?? String(value) }))}
        label="Weight"
        onChange={(value: unknown) => api.media.fonts?.ensure?.(content.font, Number(value) || 400)}
      />
    </AnimatedRow>
  {/if}
  <AnimatedRow {layer} label="Size" path="c.size"><NumField {api} {mixed} get={get('size', 0)} edit={edit('size', 'Size')} label="Size" step={1} min={4} unit="px" /></AnimatedRow>
  <AnimatedRow {layer} label="Tracking" path="c.tracking"><NumField {api} {mixed} get={get('tracking', 0)} edit={edit('tracking', 'Tracking')} label="Tracking" step={0.5} unit="px" /></AnimatedRow>
  <AnimatedRow {layer} label="Leading" path="c.leading"><NumField {api} {mixed} get={get('leading', 0)} edit={edit('leading', 'Leading')} label="Leading" step={0.02} precision={2} /></AnimatedRow>
  <AnimatedRow {layer} label="Align" path="c.align"><TextAlignment {layer} value={String(content.align ?? 'center')} /></AnimatedRow>
  <AnimatedRow {layer} label="Color" path="c.color"><ColorField {api} {mixed} get={get('color', '#F2F2F2')} edit={edit('color', 'Text color')} label="Text color" /></AnimatedRow>
  <TypeSettings {layer} family={String(content.font ?? '')} onVariableWeight={(value) => { hasVariableWeight = value; }} onStyles={(styles) => { fontStyles = styles; }} />
{:else if (layer.type === 'solid' || layer.type === 'shape' || layer.type === 'null') && !layer.d.paths?.length}
  <AnimatedRow {layer} label="Fill" path="c.color"><ColorField {api} {mixed} get={get('color', '#808080')} edit={edit('color', 'Fill')} label="Fill" /></AnimatedRow>
  {#if layer.type === 'shape'}
    <AnimatedRow {layer} label="Shape" path="c.shape"><SelectField {api} {mixed} get={get('shape', 'rect')} edit={edit('shape', 'Shape')} options={['rect', 'ellipse', 'polygon', 'star', 'line']} label="Shape" /></AnimatedRow>
  {/if}
  <AnimatedRow {layer} label="Width" path="c.w"><NumField {api} {mixed} get={get('w', 0)} edit={edit('w', 'Width')} label="Width" step={1} min={1} unit="px" /></AnimatedRow>
  <AnimatedRow {layer} label="Height" path="c.h"><NumField {api} {mixed} get={get('h', 0)} edit={edit('h', 'Height')} label="Height" step={1} min={1} unit="px" /></AnimatedRow>
  {#if layer.type === 'shape' && shape === 'rect'}
    {#snippet independentButton()}
      <button type="button" class="corner-btn" class:on={!!content.independentCorners} title="Independent corners" aria-label="Independent corners" aria-pressed={!!content.independentCorners}
        onclick={() => inspectorEdit.apply(independentPatch(!content.independentCorners), { label: 'Independent corners', origin: 'inspector' })}>
        <svg viewBox="0 0 16 16" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round"><path d={ALL_CORNERS_GLYPH}/></svg>
      </button>
    {/snippet}
    {#snippet smoothingButton()}
      <div class="corner-pop-anchor">
        <button type="button" bind:this={smoothingTrigger} class="corner-btn" class:on={smoothingOpen} class:set={!smoothingOpen && Number(content.smoothing) > 0} title="Corner smoothing" aria-label="Corner smoothing" aria-expanded={smoothingOpen}
          onclick={() => (smoothingOpen = !smoothingOpen)}>
          <svg viewBox="0 0 16 16" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round"><path d="M5 2.5v11M11 2.5v11"/><circle cx="5" cy="9.5" r="1.6" fill="currentColor" stroke="none"/><circle cx="11" cy="6.5" r="1.6" fill="currentColor" stroke="none"/></svg>
        </button>
        {#if smoothingOpen}
          <div class="corner-pop" role="dialog" aria-label="Corner smoothing" tabindex="-1" use:mountOverlayOnBody use:anchorPicker={smoothingTrigger} use:dismissOutside>
            <div class="corner-pop-head">
              <span>{cornerCompact && !content.independentCorners ? 'Corners' : 'Corner smoothing'}</span>
              <button type="button" class="corner-pop-close" aria-label="Close" onclick={() => (smoothingOpen = false)}>
                <svg viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round"><path d="M3.5 3.5l9 9M12.5 3.5l-9 9"/></svg>
              </button>
            </div>
            {#if cornerCompact && !content.independentCorners}
              <div class="corner-pop-toggle">
                <span>Independent corners</span>
                {@render independentButton()}
              </div>
            {/if}
            <div class="corner-pop-body">
              <div class="corner-slider" style={`--fill:${thumbAt(Number(content.smoothing) || 0)}`}>
                <input type="range" min="0" max="100" step="1" aria-label="Corner smoothing" value={Number(content.smoothing) || 0}
                  oninput={dragSmoothing} onchange={endSmoothing} />
                <i class="corner-ios-tick" class:hit={Number(content.smoothing) === IOS_SMOOTHING} style={`left:${thumbAt(IOS_SMOOTHING)}`}></i>
                <span class="corner-ios" style={`left:${thumbAt(IOS_SMOOTHING)}`}>iOS</span>
              </div>
              <div class="corner-pop-value">
                <NumField {api} {mixed} get={get('smoothing', 0)} edit={edit('smoothing', 'Corner smoothing')} label="Corner smoothing" ariaLabel="Corner smoothing percent" step={1} min={0} max={100} unit="%" />
              </div>
            </div>
          </div>
        {/if}
      </div>
    {/snippet}
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
        {#if !cornerCompact}{@render independentButton()}{/if}
        {@render smoothingButton()}
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
  {:else}
    <AnimatedRow {layer} label="Corner radius" path="c.radius"><NumField {api} {mixed} get={get('radius', 0)} edit={edit('radius', 'Corner radius')} label="Corner radius" step={1} min={0} unit="px" /></AnimatedRow>
  {/if}
  {#if layer.type === 'shape'}
    <AnimatedRow {layer} label="Stroke" path="c.stroke"><NumField {api} {mixed} get={get('stroke', 0)} edit={edit('stroke', 'Stroke')} label="Stroke" step={0.5} min={0} unit="px" /></AnimatedRow>
    <AnimatedRow {layer} label="Stroke color" path="c.strokeColor"><ColorField {api} {mixed} get={get('strokeColor', '#FFFFFF')} edit={edit('strokeColor', 'Stroke')} label="Stroke" /></AnimatedRow>
    {#if shape === 'polygon' || shape === 'star'}
      <AnimatedRow {layer} label="Points" path="c.points"><NumField {api} {mixed} get={get('points', 5)} edit={edit('points', 'Points')} label="Points" step={1} min={3} max={24} /></AnimatedRow>
    {/if}
  {/if}
{:else if layer.type === 'image' || layer.type === 'video'}
  <AnimatedRow {layer} label="Source"><SelectField {api} {mixed} get={get('asset', null)} edit={edit('asset', 'Source')} options={mediaOptions(layer.type)} label="Source" onChange={(value: unknown) => selectSource(layer.type, value)} /></AnimatedRow>
  <AnimatedRow {layer} label="Fit" path="c.fit"><SelectField {api} {mixed} get={get('fit', 'cover')} edit={edit('fit', 'Fit')} options={['cover', 'contain', 'stretch']} label="Fit" /></AnimatedRow>
  <AnimatedRow {layer} label="Width" path="c.w"><NumField {api} {mixed} get={get('w', 0)} edit={edit('w', 'Width')} label="Width" step={1} min={1} unit="px" /></AnimatedRow>
  <AnimatedRow {layer} label="Height" path="c.h"><NumField {api} {mixed} get={get('h', 0)} edit={edit('h', 'Height')} label="Height" step={1} min={1} unit="px" /></AnimatedRow>
  {#if layer.type === 'video'}
    <AnimatedRow {layer} label="Trim start" path="c.trim"><NumField {api} {mixed} get={get('trim', 0)} edit={edit('trim', 'Trim start')} label="Trim start" step={0.05} precision={2} unit="s" /></AnimatedRow>
    <AnimatedRow {layer} label="Speed" path="c.speed"><NumField {api} {mixed} get={get('speed', 1)} edit={edit('speed', 'Speed')} label="Speed" step={0.05} precision={2} min={0.05} /></AnimatedRow>
    {#if content.embeddedAudio === true}
      <AnimatedRow {layer} label="Mute audio"><ToggleField {api} {mixed} get={get('audioMuted', false)} edit={edit('audioMuted', 'Mute audio')} label="Mute audio" /></AnimatedRow>
    {/if}
  {/if}
{:else if layer.type === 'audio'}
  <AnimatedRow {layer} label="Trim start" path="c.trim"><NumField {api} {mixed} get={get('trim', 0)} edit={edit('trim', 'Trim start')} label="Trim start" step={0.05} precision={2} min={0} unit="s" /></AnimatedRow>
  <AnimatedRow {layer} label="Gain" path="c.gain"><NumField {api} {mixed} get={get('gain', 1)} edit={edit('gain', 'Gain')} label="Gain" step={0.05} precision={2} min={0} max={4} /></AnimatedRow>
  <AnimatedRow {layer} label="Fade in" path="c.fadeIn"><NumField {api} {mixed} get={get('fadeIn', 0)} edit={edit('fadeIn', 'Fade in')} label="Fade in" step={0.05} precision={2} min={0} max={layer.dur} unit="s" /></AnimatedRow>
  <AnimatedRow {layer} label="Fade out" path="c.fadeOut"><NumField {api} {mixed} get={get('fadeOut', 0)} edit={edit('fadeOut', 'Fade out')} label="Fade out" step={0.05} precision={2} min={0} max={layer.dur} unit="s" /></AnimatedRow>
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

<style>
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
  .corner-btn.set { color: var(--accent-tx); }
  .corner-pop-anchor { position: relative; flex: none; }
  .corner-pop {
    position: fixed; z-index: 470; width: 240px; overflow: auto;
    border-radius: var(--r-md, 8px); background: var(--bg-float, var(--bg-panel)); box-shadow: var(--shadow-float);
    outline: none;
    transform-origin: top right; animation: pm-menu-in .18s cubic-bezier(.23, 1, .32, 1) both;
  }
  @media (prefers-reduced-motion: reduce) { .corner-grid, .corner-pop { animation: none; } }
  .corner-pop-head {
    display: flex; align-items: center; justify-content: space-between; height: 40px; padding: 0 8px 0 14px;
    font-size: var(--fs-sm, var(--fs-xs)); font-weight: var(--fw-medium); color: var(--tx);
    border-bottom: 1px solid var(--line);
  }
  .corner-pop-close {
    width: 24px; height: 24px; display: grid; place-items: center; padding: 0;
    border: 0; border-radius: var(--r-sm); background: transparent; color: var(--tx-2); cursor: pointer;
  }
  .corner-pop-close:hover { background: rgb(var(--ink-rgb) / .08); color: var(--tx); }
  .corner-pop-toggle { display: flex; align-items: center; justify-content: space-between; padding: 8px 8px 0 14px; font-size: var(--fs-xs); color: var(--tx-2); }
  .corner-pop-body { display: flex; align-items: flex-start; gap: 10px; padding: 12px 14px 10px; }
  .corner-pop-value { width: 56px; flex: none; display: flex; }
  .corner-slider { position: relative; flex: 1; min-width: 0; height: 40px; }
  .corner-slider input[type=range] {
    -webkit-appearance: none; appearance: none; display: block; width: 100%; height: 28px; margin: 0;
    background: transparent; cursor: pointer;
  }
  .corner-slider input[type=range]::-webkit-slider-runnable-track {
    height: 16px; border-radius: 8px;
    background: linear-gradient(90deg, rgb(var(--ink-rgb) / .22) var(--fill), rgb(var(--ink-rgb) / .08) var(--fill));
  }
  .corner-slider input[type=range]::-webkit-slider-thumb {
    -webkit-appearance: none; width: 16px; height: 16px; border-radius: 50%;
    background: #fff; box-shadow: 0 1px 3px rgb(0 0 0 / .35);
  }
  .corner-slider input[type=range]:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; border-radius: 8px; }
  /* The tick sits on the track at the iOS value; the thumb snaps to it. */
  .corner-ios-tick {
    position: absolute; top: 12px; width: 4px; height: 4px; border-radius: 50%;
    transform: translateX(-50%); background: rgb(var(--ink-rgb) / .45); pointer-events: none;
  }
  .corner-ios-tick.hit { opacity: 0; }
  .corner-ios {
    position: absolute; bottom: 0; transform: translateX(-50%);
    font-size: var(--fs-xs); color: var(--tx-3); pointer-events: none;
  }
</style>
