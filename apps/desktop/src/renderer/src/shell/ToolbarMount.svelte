<script lang="ts">
  import { onMount } from 'svelte';

  let { PM }: { PM: Record<string, any> } = $props();
  let body: HTMLDivElement;
  let hidden = $state(false);

  function syncVisibility(): void {
    hidden = !!PM.ProjectsScreen?.isOpen;
  }

  onMount(() => {
    let disposePanel: void | (() => void);
    function mountPanel(): void {
      if (disposePanel || body.childNodes.length) return;
      const def = PM.PANELS?.toolbar;
      if (!def) return;
      try {
        disposePanel = def.build(body, {});
      } catch (error) {
        window.console.error('toolbar', error);
      }
    }

    mountPanel();
    const offProjects = PM.bus?.on?.('projects:screen', syncVisibility);
    const offLayout = PM.bus?.on?.('layout', mountPanel);
    syncVisibility();
    return () => {
      offProjects?.();
      offLayout?.();
      disposePanel?.();
    };
  });
</script>

<div id="toolbar-strip" {hidden} data-svelte-toolbar>
  <div bind:this={body}></div>
</div>
