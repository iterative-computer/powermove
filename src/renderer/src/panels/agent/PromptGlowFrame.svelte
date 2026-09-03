<script lang="ts">
  import { useFrame } from '@motion-core/motion-gpu/svelte';
  import { GLOW_PULSE_LEAD, PROMPT_GLOW_TAIL_RADIUS, glowCornerRadius, glowEntrance, glowGain, glowPad, glowPulse } from './prompt-glow-material';

  let {
    dpr,
    visible,
    onFirstFrame,
  }: {
    dpr: number;
    visible: boolean;
    onFirstFrame?: () => void;
  } = $props();

  let elapsed = 0;
  let firstFrame = true;

  const frame = useFrame((state) => {
    // Accumulating the capped delta keeps the pulse continuous across the
    // pauses the halo takes while it is off-screen or backgrounded.
    elapsed += Math.max(0, state.delta);
    state.setUniform('uElapsed', elapsed);
    state.setUniform('uPulse', glowPulse(elapsed + GLOW_PULSE_LEAD));
    state.setUniform('uIntensity', glowEntrance(elapsed));
    state.setUniform('uPad', glowPad(dpr));
    // A prompt can wrap to another line mid-run, so the shape is read per frame.
    state.setUniform('uRadius', glowCornerRadius(state.canvas.width, state.canvas.height, dpr));
    state.setUniform('uTailRadius', glowCornerRadius(state.canvas.width, state.canvas.height, dpr, PROMPT_GLOW_TAIL_RADIUS));
    // The theme can change mid-run, and the halo has to stay readable in both.
    state.setUniform('uGain', glowGain(document.documentElement.dataset.theme === 'dark'));
    if (firstFrame) {
      firstFrame = false;
      onFirstFrame?.();
    }
  });

  $effect(() => {
    if (visible) frame.start();
    else frame.stop();
  });
</script>
