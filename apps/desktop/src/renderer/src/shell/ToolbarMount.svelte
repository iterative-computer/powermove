<script lang="ts">
  import { onMount } from 'svelte';

  let { PM }: { PM: Record<string, any> } = $props();
  let strip: HTMLDivElement;
  let body: HTMLDivElement;
  let hidden = $state(false);

  function syncVisibility(): void {
    hidden = !!PM.ProjectsScreen?.isOpen;
  }

  /* The tool strip belongs to the document, not the window: it floats over the
     viewer panel so the titlebar is tabs only. The viewer element can be rebuilt
     on any layout change, so re-home the strip whenever the layout settles. */
  function placeInViewer(): void {
    const viewer = document.getElementById('panel-viewer');
    if (!viewer || strip.parentElement === viewer) return;
    viewer.appendChild(strip);
  }

  onMount(() => {
    let disposePanel: void | (() => void);
    function mountPanel(): void {
      placeInViewer();
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
    const offWorkspaces = PM.bus?.on?.('workspaces', mountPanel);
    syncVisibility();
    return () => {
      offProjects?.();
      offLayout?.();
      offWorkspaces?.();
      disposePanel?.();
      strip.remove();
    };
  });
</script>

<div id="toolbar-strip" bind:this={strip} {hidden} data-svelte-toolbar>
  <div bind:this={body}></div>
</div>
