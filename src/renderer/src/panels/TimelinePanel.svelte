<script lang="ts">
  import { onMount } from 'svelte';

  import { createTimeline } from '../legacy/ui/timeline';
  import type { PanelProps } from './registerSveltePanel';

  let { panelId: _panelId, spec: _spec }: PanelProps = $props();
  let head: HTMLElement;
  let wrap: HTMLElement;

  onMount(() => {
    const timeline = createTimeline(window.PM as Record<string, any>);
    timeline.attachHead(head);
    timeline.attachCanvas(wrap);
  });
</script>

<div id="tl-head" bind:this={head}></div>
<div id="tl-canvas-wrap" bind:this={wrap}>
  <canvas id="tl-canvas"></canvas>
</div>
