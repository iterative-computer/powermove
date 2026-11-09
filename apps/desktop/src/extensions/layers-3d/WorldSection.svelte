<script lang="ts">
  import { untrack } from 'svelte';
  import type { Layer, PowermoveAPI } from 'powermove';
  import SceneField from './SceneField.svelte';
  import type { SceneUiState } from './scene-state.svelte';
  import { SKY_PRESETS, SKY_PRESET_VALUES, WORLD_MODES, WORLD_LIGHT_FIELDS, WORLD_SUN_FIELDS, WORLD_FOG_FIELDS, domePoint, domeAngles, environmentSourceChoices, pickEnvironmentImage, sourceKey, sourceFromKey } from './environment-ui';

  /* The composition's World: sky preset, sun on a small dome, an image,
     composition or layer as the environment, and atmosphere. `edit` applies
     structural changes (mode, preset, source) for the owning layer. */
  let { api, ui, layer, world, title = 'World', edit }: {
    api: PowermoveAPI; ui: SceneUiState; layer: Layer; world: any; title?: string;
    edit: (patch: any, label: string) => void;
  } = $props();
  const { Row, Section, SelectField } = untrack(() => api.ui.controls);
  // World layers and Environment lights both keep these as `world.*` channels.
  const channelOwner = 'world';
  const value = (key: string) => (ui.version, ui.time, world?.p?.[key] ? api.anim.evP(layer, world.p[key], ui.time, `world.${key}`) : undefined);
  const R = 46;
  const sun = $derived(domePoint(Number(value('azimuth') ?? 0), Number(value('elevation') ?? 45), R));
  const assetName = (id: string) => (api.project.get().assets[id] as any)?.name ?? 'Image';
  const choices = $derived((ui.version, [
    ...(typeof world?.source === 'string' ? [{ v: `asset:${world.source}`, label: assetName(world.source) }] : []),
    { v: 'pick', label: 'Choose HDR or image…' },
    ...environmentSourceChoices(api)
  ]));
  const sourceValue = $derived(sourceKey(world?.source) || 'none');

  let dragging = $state(false);
  function sunFromPointer(event: PointerEvent, svg: SVGSVGElement): void {
    const box = svg.getBoundingClientRect(), scale = 120 / box.width;
    const { azimuth, elevation } = domeAngles((event.clientX - box.left) * scale - 60, (event.clientY - box.top) * scale - 60, R);
    const time = api.transport.time();
    api.edit.apply([
      { type: 'set_property', target: layer.id, path: 'world.azimuth', value: azimuth, time, mode: 'auto', preserveHandEdits: false },
      { type: 'set_property', target: layer.id, path: 'world.elevation', value: elevation, time, mode: 'auto', preserveHandEdits: false }
    ] as any, { origin: 'inspector' });
    api.transport.invalidate();
  }
  function down(event: PointerEvent): void {
    if (!channelOwner) return;
    const svg = event.currentTarget as SVGSVGElement;
    svg.setPointerCapture(event.pointerId); dragging = true;
    api.edit.begin('Move sun', { origin: 'inspector' });
    sunFromPointer(event, svg);
  }
  function move(event: PointerEvent): void { if (dragging) sunFromPointer(event, event.currentTarget as SVGSVGElement); }
  function up(): void {
    if (!dragging) return; dragging = false;
    const result = api.edit.commit('Move sun');
    if (!result.ok) api.ui.toast(result.message, { error: true });
  }
  function key(event: KeyboardEvent): void {
    const step = event.shiftKey ? 10 : 2;
    const delta = { ArrowLeft: ['azimuth', -step], ArrowRight: ['azimuth', step], ArrowUp: ['elevation', step], ArrowDown: ['elevation', -step] }[event.key] as [string, number] | undefined;
    if (!delta || !channelOwner) return;
    event.preventDefault();
    const next = Math.max(-90, Math.min(90, Number(value(delta[0]) ?? 0) + delta[1]));
    api.edit.apply({ type: 'set_property', target: layer.id, path: `world.${delta[0]}`, value: delta[0] === 'azimuth' ? Number(value('azimuth') ?? 0) + delta[1] : next, time: api.transport.time(), mode: 'auto', preserveHandEdits: false } as any, { label: 'Move sun', origin: 'inspector' });
    api.transport.invalidate();
  }
  async function chooseSource(next: unknown): Promise<void> {
    if (next === 'pick') { const id = await pickEnvironmentImage(api); if (id) edit({ mode: 'hdri', source: id }, 'Environment image'); return; }
    edit({ mode: 'hdri', source: sourceFromKey(String(next)) }, 'Environment source');
  }
  const gradient = (p: typeof SKY_PRESETS[number]) => `linear-gradient(180deg,${p.sky[0]} 0%,${p.sky[1]} 62%,${p.sky[2]} 100%)`;
  /* Rendered by Studio's sky model; the gradient stands in where it can't be. */
  const rendered = untrack(() => Object.fromEntries(SKY_PRESETS.map(p => {
    const v = SKY_PRESET_VALUES[p.id];
    return [p.id, api.scene3d.studio?.skyThumbnail?.({ elevation: v.elevation!, haze: v.haze!, overcast: v.overcast!, sunIntensity: v.sunIntensity! }, 96) ?? null];
  })) as Record<string, string | null>);
</script>

<Section {api} {title} />
<div class="presets" role="radiogroup" aria-label="Sky">
  {#each SKY_PRESETS as preset (preset.id)}
    {@const on = world.mode === 'sky' && world.preset === preset.id}
    <button type="button" role="radio" aria-checked={on} class="preset" class:on title={preset.label} aria-label={preset.label}
      onclick={() => edit({ mode: 'sky', preset: preset.id }, `Sky: ${preset.label}`)}>
      {#if rendered[preset.id]}
        <span class="thumb" data-rendered style={`background:center/cover url(${rendered[preset.id]})`}></span>
      {:else}
        <span class="thumb" style={`background:${gradient(preset)}`}>
          {#if preset.sun}<span class="disc" style={`background:${preset.sun};box-shadow:0 0 8px 3px ${preset.sun};bottom:${18 + preset.elevation / 3}%`}></span>{/if}
          <span class="ground"></span>
        </span>
      {/if}
      <span class="name">{preset.label}</span>
    </button>
  {/each}
</div>
<Row {api} label="Environment"><SelectField {api} get={() => world.mode} options={WORLD_MODES.map(m => ({ v: m.v, label: m.label }))} edit={{ mode: 'set', label: 'Environment', set: (v: unknown) => edit({ mode: v }, 'Environment') }} label="Environment" /></Row>
{#if world.mode === 'hdri'}
  <Row {api} label="Source"><SelectField {api} get={() => sourceValue} options={sourceValue === 'none' ? [{ v: 'none', label: 'None' }, ...choices] : choices} edit={{ mode: 'set', label: 'Environment source', set: (v: unknown) => void chooseSource(v) }} label="Environment source" /></Row>
{/if}
{#if world.mode === 'sky' && channelOwner}
  <div class="dome-row">
    <svg class="dome" class:dragging viewBox="0 0 120 120" role="slider" tabindex="0" aria-label="Sun position"
      aria-valuetext={`Direction ${Math.round(Number(value('azimuth') ?? 0))}°, height ${Number(value('elevation') ?? 0)}°`}
      aria-valuenow={Number(value('elevation') ?? 0)} aria-valuemin={-90} aria-valuemax={90}
      onpointerdown={down} onpointermove={move} onpointerup={up} onpointercancel={up} onkeydown={key}>
      <circle class="below" cx="60" cy="60" r={R * 1.2} />
      <circle class="sky" cx="60" cy="60" r={R} />
      <circle class="ring" cx="60" cy="60" r={R * 2 / 3} />
      <circle class="ring" cx="60" cy="60" r={R / 3} />
      <line class="ring" x1="60" y1={60 - R} x2="60" y2={60 + R} /><line class="ring" x1={60 - R} y1="60" x2={60 + R} y2="60" />
      <text class="label" x="60" y="117" text-anchor="middle">Front</text>
      <circle class="sun" cx={60 + sun.x} cy={60 + sun.y} r="6" />
    </svg>
    <p class="hint">Drag the sun, or drag the sky in the viewer. The centre is overhead, the edge is the horizon.</p>
  </div>
{/if}
{#if world.mode === 'sky' && channelOwner}
  {#each WORLD_SUN_FIELDS as field (field.key)}<SceneField {api} {ui} {layer} {field} path={`world.${field.key}`} channel={world.p[field.key]} />{/each}
{/if}
{#if channelOwner}
  {#each WORLD_LIGHT_FIELDS as field (field.key)}<SceneField {api} {ui} {layer} {field} path={`world.${field.key}`} channel={world.p[field.key]} />{/each}
  <Section {api} title="Atmosphere" />
  {#each WORLD_FOG_FIELDS as field (field.key)}<SceneField {api} {ui} {layer} {field} path={`world.${field.key}`} channel={world.p[field.key]} />{/each}
{/if}

<style>
  .presets { display: grid; grid-template-columns: repeat(5, minmax(0, 1fr)); gap: 6px; padding: 2px var(--panel-edge-inset, 10px) 8px; }
  .preset { display: flex; flex-direction: column; align-items: stretch; gap: 4px; padding: 0; border: 0; background: transparent; cursor: pointer; color: var(--tx-2); min-width: 0; }
  .thumb { position: relative; display: block; aspect-ratio: 1; border-radius: var(--r-sm, 6px); overflow: hidden; outline: 2px solid transparent; outline-offset: 1px; }
  .preset.on .thumb { outline-color: var(--accent); }
  .preset:focus-visible .thumb { outline-color: var(--focus-ring, var(--accent)); }
  .disc { position: absolute; left: 62%; width: 7px; height: 7px; border-radius: 50%; }
  .ground { position: absolute; left: 0; right: 0; bottom: 0; height: 18%; background: rgba(40, 38, 32, .55); }
  .name { font-size: 10px; line-height: 1.15; text-align: center; overflow-wrap: anywhere; }
  .preset.on .name { color: var(--tx); }
  .dome-row { display: flex; align-items: center; gap: 10px; padding: 2px var(--panel-edge-inset, 10px) 8px; }
  .dome { width: 112px; height: 112px; flex: none; touch-action: none; cursor: grab; border-radius: 50%; }
  .dome.dragging { cursor: grabbing; }
  .dome:focus-visible { outline: 2px solid var(--focus-ring, var(--accent)); outline-offset: 2px; }
  .below { fill: var(--bg-sunken, var(--bg-field)); }
  .sky { fill: var(--bg-field); stroke: var(--line-strong, var(--line)); stroke-width: 1; }
  .ring { fill: none; stroke: var(--line); stroke-width: .75; }
  .label { fill: var(--tx-3); font-size: 8px; font-family: var(--f-ui); }
  .sun { fill: #FFC24A; stroke: #fff; stroke-width: 1.5; }
  .hint { margin: 0; font-size: var(--fs-xs, 11px); color: var(--tx-3); line-height: 1.35; }
</style>
