<script lang="ts">
  import type { InspectorSectionDefinition, PowermoveAPI } from 'powermove';

  let { api, after, layerIds }: {
    api: PowermoveAPI;
    after: 'content' | 'transform' | 'effects';
    layerIds: string[];
  } = $props();
  let version = $state(0);
  const context = $derived({ layerIds });
  const sections = $derived.by(() => {
    void version;
    return api.inspector.sections().filter(section =>
      (section.after ?? 'transform') === after && (!section.when || section.when(context)));
  });
  $effect(() => {
    const subscription = api.events.on('inspector:changed', () => { version++; });
    return () => subscription.dispose();
  });

  function mountSection(target: HTMLElement, section: InspectorSectionDefinition) {
    const cleanup = section.build(target, { layerIds: [...layerIds] });
    return { destroy() {
      if (typeof cleanup === 'function') cleanup();
      else cleanup?.dispose();
    } };
  }
</script>

{#each sections as section (section)}
  {#key JSON.stringify(layerIds)}
    <section class="sec" data-inspector-section={section.id} aria-label={section.title}>
      <div use:mountSection={section}></div>
    </section>
  {/key}
{/each}
