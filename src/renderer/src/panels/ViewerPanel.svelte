<script lang="ts">
  import { onMount } from 'svelte';

  import { createViewer } from '../legacy/ui/viewer';
  import type { PanelProps } from './registerSveltePanel';

  let { panelId: _panelId, spec: _spec }: PanelProps = $props();
  let stage: HTMLElement;

  onMount(() => {
    createViewer(window.PM as Record<string, any>).attach(stage);
  });
</script>

<div id="stage" bind:this={stage}>
  <div id="stage-inner">
    <canvas id="gl"></canvas>
    <canvas id="overlay"></canvas>
  </div>
</div>
