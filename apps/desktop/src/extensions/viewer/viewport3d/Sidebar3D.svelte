<script lang="ts">
  import { onDestroy, untrack } from 'svelte';
  import type { ControlEditBinding, Layer, PowermoveAPI } from 'powermove';

  /* Blender's sidebar (N): the active item's transform, the view, and the 3D cursor. */
  let { api }: { api: PowermoveAPI } = $props();
  const viewport = untrack(() => api.scene3d.viewport);
  const { Row, NumField, ToggleField } = untrack(() => api.ui.controls);
  let vp = $state(untrack(() => viewport.state()));
  let tab = $state<'item' | 'view'>('item');
  let version = $state(0);
  let time = $state(untrack(() => api.transport.time()));
  let lastTime = -Infinity;
  const subscriptions = untrack(() => [
    viewport.onChange((next) => { vp = next; }),
    api.events.on('project:changed', () => { version++; }),
    api.events.on('selection', () => { version++; }),
    api.events.on('time', (next) => {
      const now = performance.now();
      if (!api.transport.playing() || now - lastTime >= 1000 / 15) { lastTime = now; time = next; }
    })
  ]);
  onDestroy(() => { for (const subscription of subscriptions) subscription.dispose(); });

  const layer = $derived.by(() => { void version; return (vp.active ? api.model.layer(vp.active) : null) as Layer | null; });
  const role = $derived(((layer?.d as any)?.definition as string | undefined)?.replace('powermove.3d.', '') ?? (layer ? 'group' : null));
  const value = (path: string) => { void version; void time; const prop = (layer?.p as any)?.[path]; return layer && prop ? api.anim.evP(layer, prop, time, path) : null; };
  const channel = (path: string, label: string): ControlEditBinding => ({
    mode: 'command', label, origin: 'inspector',
    command: (next: unknown) => ({ type: 'set_property', target: layer!.id, path, value: next as number, time: api.transport.time(),
      mode: api.scene3d.getAutoKey() ? 'keyframe' : 'auto', preserveHandEdits: false, markIntent: 'human' })
  });
  const local = (label: string, set: (value: number) => void): ControlEditBinding => ({ mode: 'local', label, set: (next: unknown) => set(Number(next)) });
  const LOCATION = [['X', 'position.x'], ['Y', 'position.y'], ['Z', 'position.z']] as const;
  const ROTATION = [['X', 'rotation.x'], ['Y', 'rotation.y'], ['Z', 'rotation']] as const;
  const SCALE = [['X', 'scale.x'], ['Y', 'scale.y'], ['Z', 'scale.z']] as const;
  const setCursor = (index: number, next: number) => { const point = [...vp.cursor] as [number, number, number]; point[index] = next; viewport.run('cursor.set', point); };
</script>

{#if vp.hasScene && vp.sidebar}
  <aside class="vp-sidebar" aria-label="3D viewport sidebar" data-viewport-ui>
    <div class="tabs" role="tablist" aria-label="Sidebar tabs">
      <button type="button" role="tab" aria-selected={tab === 'item'} onclick={() => (tab = 'item')}>Item</button>
      <button type="button" role="tab" aria-selected={tab === 'view'} onclick={() => (tab = 'view')}>View</button>
    </div>
    <div class="content" role="tabpanel">
      {#if tab === 'item'}
        {#if layer}
          <div class="name" title={layer.name}>{layer.name}</div>
          <div class="group">Location</div>
          {#each LOCATION as [axis, path] (path)}
            <Row {api} label={axis}><div class="well"><NumField {api} get={() => value(path)} edit={channel(path, `Location ${axis}`)} label={`Location ${axis}`} ariaLabel={`Location ${axis}`} step={0.1} /></div></Row>
          {/each}
          {#if role === 'object' || role === 'group'}
            <div class="group">Rotation</div>
            {#each ROTATION as [axis, path] (path)}
              <Row {api} label={axis}><div class="well"><NumField angle {api} get={() => value(path)} edit={channel(path, `Rotation ${axis}`)} label={`Rotation ${axis}`} ariaLabel={`Rotation ${axis}`} step={1} unit="°" /></div></Row>
            {/each}
            <div class="group">Scale</div>
            {#each SCALE as [axis, path] (path)}
              <Row {api} label={axis}><div class="well"><NumField {api} get={() => value(path)} edit={channel(path, `Scale ${axis}`)} label={`Scale ${axis}`} ariaLabel={`Scale ${axis}`} step={1} unit="%" /></div></Row>
            {/each}
          {/if}
        {:else}
          <p class="empty">Select a model, light or camera.</p>
        {/if}
      {:else}
        <div class="group">View</div>
        <Row {api} label="Field of view"><div class="well"><NumField angle {api} get={() => vp.view.fov} edit={local('Field of view', (next) => viewport.run('view.fov', next))} label="Field of view" ariaLabel="Viewport field of view" step={1} min={1} max={150} unit="°" /></div></Row>
        <Row {api} label="Lock camera"><ToggleField {api} get={() => vp.lockCamera} edit={{ mode: 'local', label: 'Lock camera to view', set: (next: unknown) => viewport.run('view.lock-camera', !!next) }} label="Lock camera to view" /></Row>
        <div class="group">3D Cursor</div>
        {#each ['X', 'Y', 'Z'] as axis, index (axis)}
          <Row {api} label={axis}><div class="well"><NumField {api} get={() => vp.cursor[index]} edit={local(`Cursor ${axis}`, (next) => setCursor(index, next))} label={`Cursor ${axis}`} ariaLabel={`3D cursor ${axis}`} step={0.1} /></div></Row>
        {/each}
      {/if}
    </div>
  </aside>
{/if}

<style>
  .vp-sidebar{position:absolute;right:0;top:0;bottom:0;z-index:6;width:236px;display:flex;flex-direction:column;background:color-mix(in srgb,var(--bg-panel) 94%,transparent);border-left:1px solid color-mix(in srgb,var(--tx) 10%,transparent);font:var(--fs-sm) var(--f-ui);color:var(--tx-2)}
  .tabs{display:flex;gap:2px;padding:6px 6px 0}
  .tabs button{all:unset;padding:4px 10px;border-radius:var(--r-sm) var(--r-sm) 0 0;cursor:pointer;color:var(--tx-3)}
  .tabs button[aria-selected="true"]{background:color-mix(in srgb,var(--tx) 8%,transparent);color:var(--tx)}
  .tabs button:focus-visible{outline:2px solid var(--accent);outline-offset:-2px}
  .content{flex:1;overflow:auto;padding:6px 8px 12px;border-top:1px solid color-mix(in srgb,var(--tx) 8%,transparent)}
  .name{padding:4px 2px 6px;color:var(--tx);font-weight:var(--fw-medium,500);overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
  .group{margin:8px 2px 2px;color:var(--tx-3);font-size:var(--fs-xs,11px);text-transform:uppercase;letter-spacing:.04em}
  .empty{margin:10px 2px;color:var(--tx-3)}
</style>
