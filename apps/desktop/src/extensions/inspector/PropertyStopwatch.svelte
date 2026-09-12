<script lang="ts">
  import { toggleSelectionKey } from './selection-animation';
  import { inspectorContext } from './context';
  import { inspectorRefresh } from './refresh.svelte.js';
  const { doc, transport } = inspectorContext();
  let { PM, layer, path, label, fallback }: {
    PM: Record<string, any>; layer: any; path: string; label: string; fallback?: unknown;
  } = $props();
  const property = $derived((inspectorRefresh.version, doc.tick.history, doc.tick.values, doc.tick.structure, doc.proj, PM.findProp?.(layer, path)));
  const animated = $derived((inspectorRefresh.version, doc.tick.history, doc.tick.values, doc.proj, !!property?.kf?.length));
  const current = $derived((inspectorRefresh.version, doc.tick.history, doc.tick.values, transport.time, !!property && !!PM.hasKeyAt(layer, property, transport.time)));
  function toggle(event: MouseEvent): void {
    event.stopPropagation();
    const parts = path.split('.');
    const initial = path.startsWith('l.') ? layer[path.slice(2)]
      : path.endsWith('.$enabled') ? layer.fx?.find((fx: any) => fx.id === parts[0])?.on
      : path.startsWith('m.') ? layer.masks?.find((mask: any) => mask.id === parts[1])?.[parts[2]!]
      : fallback;
    const value = property ? PM.evP(layer, property, transport.time, path) : initial;
    const result=toggleSelectionKey(PM,layer,[path],transport.time,`${current?'Remove keyframe for':'Add keyframe for'} ${label}`,value);
    if (result?.ok === false) { PM.toast?.(result.message); return; }
    PM.TL?.reveal?.(layer, [path]);
    PM.invalidate?.();
    inspectorRefresh.bump();
  }
</script>

<button type="button" class="stopwatch property-stopwatch" class:on={animated} class:at-key={current}
  aria-label={`${current ? 'Remove keyframe for' : 'Add keyframe for'} ${label}`} aria-pressed={current}
  title={current ? 'Remove keyframe' : 'Add keyframe'}
  data-property-path={path} onclick={toggle}>
  <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 3 20 12 12 21 4 12Z"/></svg>
</button>
