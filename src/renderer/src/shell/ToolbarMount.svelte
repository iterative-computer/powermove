<script lang="ts">
  import { onMount } from 'svelte';

  let { PM }: { PM: Record<string, any> } = $props();
  let body: HTMLDivElement;
  let hidden = $state(false);

  function syncVisibility(): void {
    hidden = !!PM.ProjectsScreen?.isOpen;
  }

  onMount(() => {
    const def = PM.PANELS?.toolbar;
    if (def) {
      try {
        def.build(body, {});
      } catch (error) {
        window.console.error('toolbar', error);
      }
    }

    const off = PM.bus?.on?.('projects:screen', syncVisibility);
    syncVisibility();
    return () => off?.();
  });
</script>

<div id="toolbar-strip" {hidden} data-svelte-toolbar>
  <div bind:this={body}></div>
</div>
