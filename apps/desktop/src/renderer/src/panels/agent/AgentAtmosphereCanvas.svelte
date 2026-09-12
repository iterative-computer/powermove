<script lang="ts">
  import { FragCanvas } from '@motion-core/motion-gpu/svelte';
  import type { MotionGPUErrorReport } from '@motion-core/motion-gpu';
  import { GLOW_COLOR_PIPELINE } from './prompt-glow-material';
  import { atmosphereMaterial } from './agent-atmosphere-material';
  import AgentAtmosphereFrame from './AgentAtmosphereFrame.svelte';

  let { dpr, onError, onFirstFrame }: {
    dpr: number;
    onError?: (report: MotionGPUErrorReport) => void;
    onFirstFrame?: () => void;
  } = $props();

  const clearColor: [number, number, number, number] = [0, 0, 0, 0];
  const adapterOptions: GPURequestAdapterOptions = { powerPreference: 'low-power' };
</script>

<FragCanvas material={atmosphereMaterial} color={GLOW_COLOR_PIPELINE} {clearColor} {adapterOptions}
  {dpr} maxDelta={0.05} renderMode="on-demand" showErrorOverlay={false} {onError} aria-hidden="true">
  <AgentAtmosphereFrame {onFirstFrame} />
</FragCanvas>
