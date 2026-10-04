<script lang="ts">
  import { CAPTION_PRESETS, normalizeCaptionStyle, presetStyle, type CaptionStyle } from 'powermove';
  import { inspectorContext, type EditBinding } from './context';

  /* Captions layer properties: a preset to start from, then the type, plate,
     placement and word-highlight controls in the inspector's row grammar.
     A change applies to every selected captions layer. */
  let { layer, fontsVersion = 0 }: { layer: any; fontsVersion?: number } = $props();
  const { api, doc, sel, edit: inspectorEdit } = inspectorContext();
  const { ColorField, FontField, NumField, Row, Section, SelectField, ToggleField } = api.ui.controls;

  const style = $derived((doc.tick.values, doc.tick.structure, doc.proj, normalizeCaptionStyle(layer.d?.style)) as CaptionStyle);
  const targets = $derived((sel.layers, doc.tick.structure, sel.layers
    .map((id) => api.model.layer(id) as any)
    .filter((item) => item?.type === 'captions')
    .map((item) => item.id as string)));
  const ids = () => (targets.includes(layer.id) ? targets : [layer.id]);
  const presets = $derived(CAPTION_PRESETS.map((preset) => ({ ...preset, look: presetStyle(preset.id, 1080) })));

  const styleCommands = (patch: Partial<CaptionStyle>) => ids().map((target) => ({
    type: 'edit_captions' as const, target, op: 'style' as const, style: patch as any
  }));
  const bind = (key: keyof CaptionStyle, label: string, map: (value: unknown) => unknown = (value) => value): EditBinding => ({
    mode: 'command', label, origin: 'inspector',
    command: (value: unknown) => styleCommands({ [key]: map(value) } as Partial<CaptionStyle>)
  });
  const get = (key: keyof CaptionStyle) => () => style[key];
  const mixed = (binding: EditBinding, value: unknown) => {
    if (binding.mode !== 'command' || targets.length < 2) return false;
    const label = binding.label;
    const key = Object.entries(BINDINGS).find(([, item]) => item.label === label)?.[0] as keyof CaptionStyle | undefined;
    if (!key) return false;
    return targets.some((id) => normalizeCaptionStyle((api.model.layer(id) as any)?.d?.style)[key] !== value);
  };

  function applyPreset(id: string) {
    if (style.preset === id && targets.length < 2) return;
    inspectorEdit.apply(styleCommands({ preset: id }), { label: 'Caption style', origin: 'inspector' });
    api.transport.invalidate?.('render');
  }

  const weightLabels: Record<number, string> = { 300: 'Light', 400: 'Regular', 500: 'Medium', 600: 'Semibold', 700: 'Bold', 800: 'Heavy', 900: 'Black' };
  const weights = $derived([...new Set([style.weight, 300, 400, 500, 600, 700, 800, 900])].sort((a, b) => a - b)
    .map((value) => ({ v: value, label: weightLabels[value] ?? String(value) })));

  const BINDINGS = {
    font: bind('font', 'Caption font'),
    weight: bind('weight', 'Caption weight', Number),
    size: bind('size', 'Caption size', Number),
    fill: bind('fill', 'Caption color'),
    stroke: bind('stroke', 'Caption outline', Number),
    strokeColor: bind('strokeColor', 'Outline color'),
    textCase: bind('textCase', 'Text case'),
    box: bind('box', 'Background'),
    boxColor: bind('boxColor', 'Background color'),
    boxOpacity: bind('boxOpacity', 'Background opacity', Number),
    boxPadding: bind('boxPadding', 'Background padding', Number),
    boxRadius: bind('boxRadius', 'Background corner radius', Number),
    placement: bind('placement', 'Caption position'),
    align: bind('align', 'Caption alignment'),
    offsetY: bind('offsetY', 'Vertical offset', Number),
    offsetX: bind('offsetX', 'Horizontal offset', Number),
    safeMargin: bind('safeMargin', 'Safe margin', Number),
    maxWidth: bind('maxWidth', 'Maximum width', Number),
    maxLines: bind('maxLines', 'Maximum lines', Number),
    highlight: bind('highlight', 'Word highlight'),
    highlightColor: bind('highlightColor', 'Highlight color'),
    highlightScale: bind('highlightScale', 'Highlight size', Number),
    wordAnimation: bind('wordAnimation', 'Word animation')
  } satisfies Partial<Record<keyof CaptionStyle, EditBinding>>;

  /* Preview tiles draw each preset as it renders: outline, plate, case and
     the highlighted word. */
  const tileStyle = (look: CaptionStyle) => [
    `--cap-fill:${look.fill}`,
    `--cap-weight:${look.weight}`,
    `--cap-stroke:${look.stroke > 0 ? Math.max(1, look.stroke / 3) : 0}px`,
    `--cap-stroke-color:${look.strokeColor}`,
    `--cap-box:${look.box ? hexAlpha(look.boxColor, look.boxOpacity / 100) : 'transparent'}`,
    `--cap-highlight:${look.highlight ? look.highlightColor : look.fill}`,
    `text-transform:${look.textCase === 'upper' ? 'uppercase' : look.textCase === 'lower' ? 'lowercase' : 'none'}`
  ].join(';');
  function hexAlpha(hex: string, alpha: number) {
    const value = hex.replace('#', '');
    const [r, g, b] = [0, 2, 4].map((at) => parseInt(value.slice(at, at + 2), 16));
    return `rgba(${r},${g},${b},${alpha})`;
  }
</script>

<Section {api} title="Captions" />
<div class="caption-presets" role="radiogroup" aria-label="Caption style">
  {#each presets as preset (preset.id)}
    <button type="button" role="radio" class="caption-preset" aria-checked={style.preset === preset.id}
      title={preset.description} onclick={() => applyPreset(preset.id)}>
      <span class="caption-preview" style={tileStyle(preset.look)}>
        <span class="caption-sample">Say <em>it</em></span>
      </span>
      <span class="caption-name">{preset.label}</span>
    </button>
  {/each}
</div>

<Row {api} label="Font">
  {#key fontsVersion}
    <FontField {api} {mixed} get={get('font')} edit={BINDINGS.font} label="Caption font" weight={() => style.weight} />
  {/key}
</Row>
<Row {api} label="Weight"><SelectField {api} {mixed} get={get('weight')} edit={BINDINGS.weight} options={weights} label="Caption weight" /></Row>
<Row {api} label="Size"><NumField {api} {mixed} get={get('size')} edit={BINDINGS.size} label="Caption size" step={1} min={4} unit="px" /></Row>
<Row {api} label="Color"><ColorField {api} {mixed} get={get('fill')} edit={BINDINGS.fill} label="Caption color" /></Row>
<Row {api} label="Outline"><NumField {api} {mixed} get={get('stroke')} edit={BINDINGS.stroke} label="Caption outline" step={0.5} min={0} unit="px" /></Row>
{#if style.stroke > 0}
  <Row {api} label="Outline color"><ColorField {api} {mixed} get={get('strokeColor')} edit={BINDINGS.strokeColor} label="Outline color" /></Row>
{/if}
<Row {api} label="Case">
  <SelectField {api} {mixed} get={get('textCase')} edit={BINDINGS.textCase} label="Text case"
    options={[{ v: 'original', label: 'As typed' }, { v: 'upper', label: 'Uppercase' }, { v: 'lower', label: 'Lowercase' }]} />
</Row>

<Section {api} title="Background" />
<Row {api} label="Background"><ToggleField {api} {mixed} get={get('box')} edit={BINDINGS.box} label="Background" /></Row>
{#if style.box}
  <Row {api} label="Color"><ColorField {api} {mixed} get={get('boxColor')} edit={BINDINGS.boxColor} label="Background color" /></Row>
  <Row {api} label="Opacity"><NumField {api} {mixed} get={get('boxOpacity')} edit={BINDINGS.boxOpacity} label="Background opacity" step={1} min={0} max={100} unit="%" /></Row>
  <Row {api} label="Padding"><NumField {api} {mixed} get={get('boxPadding')} edit={BINDINGS.boxPadding} label="Background padding" step={1} min={0} unit="px" /></Row>
  <Row {api} label="Corner radius"><NumField {api} {mixed} get={get('boxRadius')} edit={BINDINGS.boxRadius} label="Background corner radius" step={1} min={0} unit="px" /></Row>
{/if}

<Section {api} title="Placement" />
<Row {api} label="Position">
  <SelectField {api} {mixed} get={get('placement')} edit={BINDINGS.placement} label="Caption position"
    options={[{ v: 'bottom', label: 'Bottom' }, { v: 'middle', label: 'Middle' }, { v: 'top', label: 'Top' }]} />
</Row>
<Row {api} label="Align">
  <SelectField {api} {mixed} get={get('align')} edit={BINDINGS.align} label="Caption alignment"
    options={[{ v: 'left', label: 'Left' }, { v: 'center', label: 'Center' }, { v: 'right', label: 'Right' }]} />
</Row>
<Row {api} label="Offset" pair>
  <NumField {api} {mixed} get={get('offsetX')} edit={BINDINGS.offsetX} label="Horizontal offset" ariaLabel="Horizontal offset" step={1} unit="px" />
  <NumField {api} {mixed} get={get('offsetY')} edit={BINDINGS.offsetY} label="Vertical offset" ariaLabel="Vertical offset" step={1} unit="px" />
</Row>
<Row {api} label="Safe margin"><NumField {api} {mixed} get={get('safeMargin')} edit={BINDINGS.safeMargin} label="Safe margin" step={0.5} min={0} max={45} unit="%" /></Row>
<Row {api} label="Max width"><NumField {api} {mixed} get={get('maxWidth')} edit={BINDINGS.maxWidth} label="Maximum width" step={1} min={10} max={100} unit="%" /></Row>
<Row {api} label="Max lines"><NumField {api} {mixed} get={get('maxLines')} edit={BINDINGS.maxLines} label="Maximum lines" step={1} min={1} max={6} /></Row>

<Section {api} title="Words" />
<Row {api} label="Highlight"><ToggleField {api} {mixed} get={get('highlight')} edit={BINDINGS.highlight} label="Word highlight" /></Row>
{#if style.highlight}
  <Row {api} label="Color"><ColorField {api} {mixed} get={get('highlightColor')} edit={BINDINGS.highlightColor} label="Highlight color" /></Row>
  <Row {api} label="Size"><NumField {api} {mixed} get={get('highlightScale')} edit={BINDINGS.highlightScale} label="Highlight size" step={1} min={50} max={300} unit="%" /></Row>
{/if}
<Row {api} label="Animation">
  <SelectField {api} {mixed} get={get('wordAnimation')} edit={BINDINGS.wordAnimation} label="Word animation"
    options={[{ v: 'none', label: 'None' }, { v: 'pop', label: 'Pop in' }, { v: 'fade', label: 'Fade in' }]} />
</Row>

<style>
  .caption-presets {
    display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 6px;
    padding: 4px 0 10px;
  }
  .caption-preset {
    display: flex; flex-direction: column; gap: 4px; min-width: 0; padding: 3px; border: 0;
    border-radius: var(--r-md, 8px); background: transparent; color: var(--tx-3); cursor: default;
    transition: background var(--dur-1), color var(--dur-1);
  }
  .caption-preset:hover { background: var(--ink-1); color: var(--tx-2); }
  .caption-preset[aria-checked="true"] { background: var(--bg-float); color: var(--tx); box-shadow: var(--ctl-edge); }
  .caption-preset:focus-visible { outline: 0; box-shadow: 0 0 0 2px var(--ink-2); }
  .caption-preview {
    position: relative; display: grid; place-items: end center; height: 44px; padding-bottom: 6px; overflow: hidden;
    border-radius: calc(var(--r-md, 8px) - 3px);
    background: linear-gradient(160deg, #4a5560, #23282e 70%);
  }
  .caption-sample {
    padding: 1px 5px; border-radius: 3px; background: var(--cap-box);
    color: var(--cap-fill); font-size: 12px; font-weight: var(--cap-weight); line-height: 1.2; white-space: nowrap;
    -webkit-text-stroke: var(--cap-stroke) var(--cap-stroke-color); paint-order: stroke fill;
  }
  .caption-sample em { font-style: normal; color: var(--cap-highlight); }
  .caption-name { font-size: var(--fs-xs); line-height: 14px; text-align: center; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
</style>
