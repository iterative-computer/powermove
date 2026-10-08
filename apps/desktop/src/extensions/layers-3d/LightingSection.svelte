<script lang="ts">
  import { untrack } from 'svelte';
  import type { ChannelValue, ControlEditBinding, PowermoveAPI } from 'powermove';
  import LightBall from './LightBall.svelte';
  import { LIGHT_TYPES } from './scene-model';
  import type { LightType } from './scene-types';
  import type { SceneUiState } from './scene-state.svelte';

  /* The composition's lighting in one place: aim on the ball, pick a preset, set each light's color and strength. */
  let { api, ui, focus = null }: { api: PowermoveAPI; ui: SceneUiState; focus?: string | null } = $props();
  const { Row, Section, SelectField, ColorField, NumField } = untrack(() => api.ui.controls);

  const lights = $derived((ui.version, ui.time, api.scene3d.lighting.list()));
  const camera = $derived.by(() => {
    void ui.version;
    return api.model.curComp().layers.find((l: any) => l.d?.definition === 'powermove.3d.camera' && api.anim.active(l, ui.time)) ?? null;
  });
  const environment = $derived((camera?.d as any)?.data?.environment);

  function channel(id: string, path: string, label: string): ControlEditBinding {
    return { mode: 'command', label, origin: 'inspector', command: (next: unknown) => ({
      type: 'set_property', target: id, path, value: next as ChannelValue, time: api.transport.time(),
      mode: api.scene3d.getAutoKey() ? 'keyframe' : 'auto', preserveHandEdits: false, markIntent: 'human'
    }) };
  }
  const evaluate = (layer: any, prop: any, path: string) => prop ? api.anim.evP(layer, prop, ui.time, path) : undefined;
  function report(result: { ok: boolean; message: string }) { if (!result.ok) api.ui.toast(result.message, { error: true }); api.transport.invalidate(); }

  /** A new light at `direction` from the composition centre, aimed at it. */
  function addLight(type: LightType, direction: [number, number, number] = [-.45, -.55, -.7]) {
    const comp = api.model.curComp(), cx = comp.w / 2, cy = comp.h / 2, distance = Math.max(comp.w, comp.h) * 1.4;
    const name = `${LIGHT_TYPES.find((t) => t.id === type)?.label ?? 'Sun'} light`;
    report(api.scene3d.edit({ operation: 'add_light', light: { type, name, p: {
      x: cx + direction[0] * distance, y: cy + direction[1] * distance, z: direction[2] * distance, targetX: cx, targetY: cy, targetZ: 0
    } } }, { label: `Add ${name}`, origin: 'inspector' }));
  }
  const preset = (id: unknown) => { if (id !== 'custom') report(api.scene3d.lighting.applyPreset(String(id))); };
</script>

<Section {api} title="Lighting" />
<Row {api} label="Direction">
  <LightBall {api} {lights} {focus} add={(direction) => addLight('sun', direction)} />
</Row>
<Row {api} label="Preset">
  <SelectField {api} label="Lighting preset" get={() => 'custom'}
    options={[{ v: 'custom', label: lights.length ? 'Custom' : 'None' }, ...api.scene3d.lighting.presets.map((p) => ({ v: p.id, label: p.label }))]}
    edit={{ mode: 'set', label: 'Lighting preset', set: preset }} />
</Row>
{#each lights as light (light.id)}
  {@const layer = api.model.layer(light.id) as any}
  {#if layer}
    <Row {api} label={light.name} pair onLabel={() => api.selection.select([light.id])}>
      <ColorField {api} label={`${light.name} color`} get={() => evaluate(layer, layer.d.data.light.p.color, 'light.color')} edit={channel(light.id, 'light.color', 'Light color')} />
      <NumField {api} label={`${light.name} strength`} ariaLabel={`${light.name} strength`} get={() => evaluate(layer, layer.d.data.light.p.intensity, 'light.intensity')} min={0} step={0.05} edit={channel(light.id, 'light.intensity', 'Light strength')} />
    </Row>
  {/if}
{/each}
{#if environment && camera}
  <Row {api} label="Ambient" pair>
    <ColorField {api} label="Ambient color" get={() => evaluate(camera, environment.p?.ambientColor, 'environment.ambientColor')} edit={channel(camera.id, 'environment.ambientColor', 'Ambient color')} />
    <NumField {api} label="Ambient strength" ariaLabel="Ambient strength" get={() => evaluate(camera, environment.p?.ambient, 'environment.ambient')} min={0} step={0.01} edit={channel(camera.id, 'environment.ambient', 'Ambient strength')} />
  </Row>
{/if}
<Row {api} label="Add">
  <button type="button" class="chip" onclick={(event) => api.ui.menu(event.currentTarget as HTMLElement, LIGHT_TYPES.map((t) => ({ label: `${t.label} light`, icon: 'sun', run: () => addLight(t.id) })))}>Light…</button>
</Row>
