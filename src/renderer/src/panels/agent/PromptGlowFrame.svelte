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
    const inset = Boolean(state.canvas.closest('[data-prompt-inset]'));
    state.setUniform('uPad', inset ? 4 * dpr : glowPad(dpr));
    // A prompt can wrap to another line mid-run, so the shape is read per frame.
    state.setUniform('uRadius', inset ? 8 * dpr : glowCornerRadius(state.canvas.width, state.canvas.height, dpr));
    state.setUniform('uTailRadius', inset ? 8 * dpr : glowCornerRadius(state.canvas.width, state.canvas.height, dpr, PROMPT_GLOW_TAIL_RADIUS));
    // The theme can change mid-run, and the halo has to stay readable in both.
    state.setUniform('uGain', glowGain(document.documentElement.dataset.theme === 'dark') * (inset ? 1.8 : 1));
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
