<script lang="ts">
  import { untrack } from 'svelte';
  import type { ControlEditBinding, Layer, PowermoveAPI } from 'powermove';
  import type { SceneUiState } from './scene-state.svelte';
  import SceneField from './SceneField.svelte';
  import SurfaceMaterials from './SurfaceMaterials.svelte';
  import MaterialOptionsRows from './MaterialOptionsRows.svelte';
  import {
    BEVEL_STYLES, GEOMETRY_FIELDS, GEOMETRY_TIPS, QUALITY_OPTIONS, geometryChannel, geometryPath, isExtruded,
    setExtruded, setGeometryChoice, type BevelStyle
  } from './extrusion';

  /* Geometry Options, as in After Effects' Advanced 3D: one Extrude switch on
     every text and shape layer (it makes a 2D layer 3D in the same step), then
     depth, bevel and quality once it's on, and the per-surface Material Options. */
  let { api, ui, layerId }: { api: PowermoveAPI; ui: SceneUiState; layerId: string } = $props();
  const { Row, Section, ToggleField, SelectField } = untrack(() => api.ui.controls);

  /* The layer object keeps its identity across edits; read its options through ui.version. */
  const materialOptionsOf = (l: any): any => (ui.version, l?.d?.materialOptions);
  const layer = $derived((ui.version, api.model.layer(layerId)) as Layer | null);
  const extruded = $derived((ui.version, isExtruded(layer)));
  const flat = $derived((ui.version, !(layer as any)?.threeD));
  /* Outlines load asynchronously; poll briefly so a missing font is named as soon as it is known. */
  let fontTick = $state(0);
  const fontProblem = $derived((ui.version, fontTick, (layer as any)?.type === 'text' && extruded ? api.scene3d.extrusionFontStatus(layerId).message : null));
  $effect(() => { if (!extruded || (layer as any)?.type !== 'text') return; const timer = window.setInterval(() => { fontTick++; if (api.scene3d.extrusionFontStatus(layerId).state !== 'loading') window.clearInterval(timer); }, 300); return () => window.clearInterval(timer); });
  const extrudeEdit: ControlEditBinding = { mode: 'set', label: 'Extrude', set: (on: unknown) => { setExtruded(api, layerId, !!on); } };

  const styleChannel = $derived((ui.version, geometryChannel(layer, 'bevelStyle')));
  const style = $derived((ui.version, ui.time, (layer && styleChannel ? api.anim.evP(layer, styleChannel, ui.time, geometryPath('bevelStyle')) : 'none') as BevelStyle));
  const styleAnimated = $derived((ui.version, !!styleChannel?.kf?.length));
  const styleAtKey = $derived((ui.version, ui.time, !!layer && !!styleChannel && !!api.anim.hasKeyAt(layer, styleChannel, ui.time)));
  const beveled = $derived(style !== 'none');

  const qualityChannel = $derived((ui.version, geometryChannel(layer, 'quality')));
  const qualityEdit: ControlEditBinding = { mode: 'set', label: 'Geometry Quality', set: (value: unknown) => { setGeometryChoice(api, layerId, 'quality', String(value), 'Geometry Quality'); } };

  function pickStyle(next: BevelStyle): void {
    if (next === style) return;
    setGeometryChoice(api, layerId, 'bevelStyle', next, 'Bevel Style');
  }
  function styleKey(event: MouseEvent): void {
    event.stopPropagation();
    if (!layer || !styleChannel) return;
    const time = api.transport.time(), path = geometryPath('bevelStyle');
    const label = `${styleAtKey ? 'Remove keyframe for' : 'Add keyframe for'} Bevel Style`;
    if (!styleAnimated) api.edit.apply({ type: 'set_property', target: layerId, path, value: style, time, mode: 'keyframe', preserveHandEdits: false }, { label, origin: 'inspector' });
    else api.history.do(label, () => {
      const key = api.anim.hasKeyAt(layer, styleChannel, time);
      if (key) api.anim.removeKey(styleChannel, key);
      else api.anim.setKeyOn(styleChannel, time - layer.from, style, 'linear', api.project.get().fps);
      api.anim.touch();
    });
    api.transport.invalidate();
  }
  function styleKeydown(event: KeyboardEvent, index: number): void {
    const step = event.key === 'ArrowLeft' || event.key === 'ArrowUp' ? -1 : event.key === 'ArrowRight' || event.key === 'ArrowDown' ? 1 : 0;
    if (!step) return;
    event.preventDefault();
    const next = BEVEL_STYLES[(index + step + BEVEL_STYLES.length) % BEVEL_STYLES.length]!;
    pickStyle(next.id);
    (event.currentTarget as HTMLElement).parentElement?.querySelectorAll<HTMLButtonElement>('button')[BEVEL_STYLES.indexOf(next)]?.focus();
  }
</script>

{#if layer}
  <div class="extrusion" data-extrusion-section>
    <Section {api} title="Geometry Options" />
    <div class="extrude-row" class:flat title={flat ? GEOMETRY_TIPS.extrude2D : GEOMETRY_TIPS.extrude} data-extrude-toggle>
      <Row {api} label="Extrude">
        <ToggleField {api} get={() => extruded} edit={extrudeEdit} label="Extrude" />
      </Row>
      {#if flat && !extruded}<span class="needs-3d" aria-hidden="true">{@html api.ui.icon('cube')}</span>{/if}
    </div>
    {#if extruded && fontProblem}<p class="font-problem" role="alert" data-extrusion-font-problem>{fontProblem}</p>{/if}
    {#if extruded}
      {@const depth = GEOMETRY_FIELDS[0]!}
      <div title={depth.tip}><SceneField {api} {ui} {layer} field={depth} path={geometryPath(depth.key)} channel={geometryChannel(layer, depth.key)} /></div>
      <div role="group" aria-label="Bevel Style property" data-geometry-property="geo.bevelStyle" title={GEOMETRY_TIPS.bevelStyle}>
        <Row {api} label="Bevel Style">
          {#snippet left()}
            <button type="button" class="stopwatch property-stopwatch" class:on={styleAnimated} class:at-key={styleAtKey}
              aria-label={`${styleAtKey ? 'Remove keyframe for' : 'Add keyframe for'} Bevel Style`} aria-pressed={styleAtKey}
              title={styleAtKey ? 'Remove keyframe' : styleAnimated ? 'Add keyframe' : 'Animate Bevel Style'} onclick={styleKey}>
              {@html api.ui.icon(styleAtKey ? 'diamondFill' : 'diamond')}
            </button>
          {/snippet}
          <div class="bevel-styles" role="radiogroup" aria-label="Bevel Style">
            {#each BEVEL_STYLES as option, index (option.id)}
              <button type="button" role="radio" class:on={option.id === style} aria-checked={option.id === style} aria-label={option.label}
                tabindex={option.id === style ? 0 : -1} title={option.tip} data-bevel-style={option.id}
                onclick={() => pickStyle(option.id)} onkeydown={(event) => styleKeydown(event, index)}>
                <svg viewBox="0 0 16 16" aria-hidden="true"><path d={option.path} /></svg>
              </button>
            {/each}
          </div>
        </Row>
      </div>
      {#each GEOMETRY_FIELDS.slice(1) as field (field.key)}
        <div class:inactive={!beveled} title={beveled ? field.tip : `${field.tip} Choose a bevel style to use it.`}>
          <SceneField {api} {ui} {layer} {field} path={geometryPath(field.key)} channel={geometryChannel(layer, field.key)} />
        </div>
      {/each}
      <div title={GEOMETRY_TIPS.quality} data-geometry-property="geo.quality">
        <Row {api} label="Quality">
          <SelectField {api} get={() => (ui.version, qualityChannel?.v ?? 'standard')} options={QUALITY_OPTIONS as any} edit={qualityEdit} label="Geometry Quality" />
        </Row>
      </div>
    {/if}
  </div>
  {#if extruded}<SurfaceMaterials {api} {ui} {layer} /><MaterialOptionsRows {api} {ui} {layer} options={materialOptionsOf(layer)} fallback={(key) => key === 'castsShadows' ? 'on' : true} heading={false} />{/if}
{/if}

<style>
  .extrusion { display: flex; flex-direction: column; }
  .extrude-row { position: relative; }
  .font-problem { margin: 2px 0 6px; padding: 8px 10px; border-radius: var(--r-sm); background: color-mix(in srgb, var(--danger, #E5484D) 10%, transparent); color: var(--tx); font-size: 11px; line-height: 15px; }
  /* A 2D layer wears a small cube beside the switch: turning it on makes the layer 3D. */
  .needs-3d { position: absolute; top: 50%; left: 86px; display: grid; place-items: center; width: 13px; height: 13px; margin-top: -6.5px; color: var(--tx-3); pointer-events: none; }
  .needs-3d :global(svg) { width: 12px; height: 12px; }
  .inactive :global(.row.split > .k), .inactive :global(.num) { color: var(--tx-3); }
  /* The Extrude switch's material: a field-colored track with a panel-colored
     pill under the chosen profile. */
  .bevel-styles { display: grid; grid-auto-flow: column; grid-auto-columns: minmax(0, 1fr); flex: 1 1 0; min-width: 0; height: 26px; padding: 2px; box-sizing: border-box; border-radius: var(--r-sm); background: var(--bg-field); }
  .bevel-styles button { display: grid; place-items: center; height: 22px; padding: 0; border: 0; border-radius: calc(var(--r-sm) - 2px); background: transparent; color: var(--tx-3); cursor: default; transition: color var(--dur-1), background var(--dur-1); }
  .bevel-styles button:hover { color: var(--tx-2); }
  .bevel-styles button.on { background: var(--bg-panel); color: var(--tx); }
  .bevel-styles button:active { transform: scale(.94); transition: none; }
  .bevel-styles button:focus-visible { outline: 2px solid var(--accent); outline-offset: -2px; }
  .bevel-styles svg { width: 14px; height: 14px; fill: currentColor; }
</style>
