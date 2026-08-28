<script lang="ts">
  import { FragCanvas } from '@motion-core/motion-gpu/svelte';
  import type { MotionGPUErrorReport } from '@motion-core/motion-gpu';
  import RippleFrame from './RippleFrame.svelte';
  import { rippleMaterial } from './ripple-material';

  let {
    origin,
    sceneBitmap,
    onError,
    onFirstFrame,
    onSettled,
  }: {
    origin: { x: number; y: number };
    sceneBitmap: ImageBitmap | null;
    onError?: (report: MotionGPUErrorReport) => void;
    onFirstFrame?: () => void;
    onSettled?: () => void;
  } = $props();

  const clearColor: [number, number, number, number] = [0, 0, 0, 0];
  const color = {
    dynamicRange: 'auto' as const,
    canvasColorSpace: 'display-p3' as const,
    outputEncoding: 'srgb' as const,
  };
  const adapterOptions: GPURequestAdapterOptions = { powerPreference: 'high-performance' };
  const dpr = Math.min(window.devicePixelRatio || 1, 2);
</script>

<FragCanvas
  material={rippleMaterial}
  {clearColor}
  {color}
  {adapterOptions}
  {dpr}
  maxDelta={0.05}
  renderMode="on-demand"
  showErrorOverlay={false}
  {onError}
  class="spatial-ripple-canvas"
  aria-hidden="true"
  data-ripple-canvas
>
  <RippleFrame {origin} {sceneBitmap} {onFirstFrame} {onSettled} />
</FragCanvas>
