<script lang="ts">
  import { FragCanvas } from '@motion-core/motion-gpu/svelte';
  import type { MotionGPUErrorReport } from '@motion-core/motion-gpu';
  import GhostFieldFrame from './GhostFieldFrame.svelte';
  import { GHOST_COLOR_PIPELINE, ghostMaterial } from './ghost-field-material';

  let {
    dpr,
    onError,
    onFirstFrame,
  }: {
    dpr: number;
    onError?: (report: MotionGPUErrorReport) => void;
    onFirstFrame?: () => void;
  } = $props();

  let canvas = $state<HTMLCanvasElement | undefined>();
  let visible = $state(true);

  const clearColor: [number, number, number, number] = [0, 0, 0, 0];
  // Idle chrome next to a live editor: the field never asks for the discrete GPU.
  const adapterOptions: GPURequestAdapterOptions = { powerPreference: 'low-power' };

  // A generation can outlive the view that owns it. Scrolled away or with the
  // window in the background, the field stops costing frames instead of
  // animating where nobody can see it.
  $effect(() => {
    const element = canvas;
    if (!element) return;
    let onScreen = true;
    const update = () => { visible = onScreen && !document.hidden; };
    const intersection = typeof IntersectionObserver === 'undefined' ? null : new IntersectionObserver(entries => {
      onScreen = entries.some(entry => entry.isIntersecting);
      update();
    });
    intersection?.observe(element);
    document.addEventListener('visibilitychange', update);
    update();
    return () => {
      intersection?.disconnect();
      document.removeEventListener('visibilitychange', update);
    };
  });
</script>

<FragCanvas
  material={ghostMaterial}
  {clearColor}
  color={GHOST_COLOR_PIPELINE}
  {adapterOptions}
  {dpr}
  bind:canvas
  maxDelta={0.05}
  renderMode="on-demand"
  showErrorOverlay={false}
  {onError}
  class="ghost-edge-canvas"
  aria-hidden="true"
  data-ghost-canvas
>
  <GhostFieldFrame {dpr} {visible} {onFirstFrame} />
</FragCanvas>
