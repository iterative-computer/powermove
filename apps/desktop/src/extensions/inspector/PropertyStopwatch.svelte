<script lang="ts">
  import { toggleSelectionKey } from './selection-animation';
  import { inspectorContext } from './context';
  import { inspectorRefresh } from './refresh.svelte.js';
  import { rawAt, staticValue } from './figma-data';
  const { api, doc, transport, timeline } = inspectorContext();
  /* `paths` keys several channels as one property (a gradient's angle is its
     whole transform). */
  let { layer, path, paths, label, fallback }: {
    layer: any; path: string; paths?: string[]; label: string; fallback?: unknown;
  } = $props();
  const all = $derived(paths?.length ? paths : [path]);
  const nested = $derived(all.some((key) => key.startsWith('c.') && key.slice(2).includes('.')));
  const properties = $derived((inspectorRefresh.version, doc.tick.history, doc.tick.values, doc.tick.structure, doc.proj, all.map((key) => api.anim.findProp?.(layer, key))));
  const property = $derived(properties[0]);
  const animated = $derived((inspectorRefresh.version, doc.tick.history, doc.tick.values, doc.proj, properties.some((prop) => !!prop?.kf?.length)));
  const current = $derived((inspectorRefresh.version, doc.tick.history, doc.tick.values, transport.time, properties.every((prop) => !!prop && !!api.anim.hasKeyAt(layer, prop, transport.time))));
  function toggle(event: MouseEvent): void {
    event.stopPropagation();
    if (nested) {
      // Each selected layer keys its own static value at every nested path.
      const own = (target: { layer: any; path: string }) => staticValue(rawAt(target.layer, target.path.slice(2)));
      const result = toggleSelectionKey(api, layer, all, transport.time, `${current ? 'Remove keyframe for' : 'Add keyframe for'} ${label}`, own);
      if (result?.ok === false) { api.ui.toast?.(result.message); return; }
      timeline()?.reveal(layer, all);
      api.transport.invalidate?.();
      inspectorRefresh.bump();
      return;
    }
    const parts = path.split('.');
    const initial = path.startsWith('l.') ? layer[path.slice(2)]
      : path.endsWith('.$enabled') ? layer.fx?.find((fx: any) => fx.id === parts[0])?.on
      : path.startsWith('m.') ? layer.masks?.find((mask: any) => mask.id === parts[1])?.[parts[2]!]
      : fallback;
    const value = property ? api.anim.evP(layer, property, transport.time, path) : initial;
    const result=toggleSelectionKey(api,layer,[path],transport.time,`${current?'Remove keyframe for':'Add keyframe for'} ${label}`,value);
    if (result?.ok === false) { api.ui.toast?.(result.message); return; }
    timeline()?.reveal(layer, [path]);
    api.transport.invalidate?.();
    inspectorRefresh.bump();
  }
</script>

<button type="button" class="stopwatch property-stopwatch" class:on={animated} class:at-key={current}
  aria-label={`${current ? 'Remove keyframe for' : 'Add keyframe for'} ${label}`} aria-pressed={current}
  title={current ? 'Remove keyframe' : 'Add keyframe'}
  data-property-path={path} onclick={toggle}>
  {@html api.ui.icon(current ? 'diamondFill' : 'diamond')}
</button>
