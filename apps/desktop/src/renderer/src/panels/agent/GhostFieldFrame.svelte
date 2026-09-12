<script lang="ts">
  import { useFrame } from '@motion-core/motion-gpu/svelte';
  import { GHOST_BREATH_LEAD, ghostBreath, ghostCornerRadius, ghostEntrance, ghostGain } from './ghost-field-material';

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
    // Accumulating the capped delta keeps the breath continuous across the
    // pauses this field takes while it is off-screen or backgrounded.
    elapsed += Math.max(0, state.delta);
    state.setUniform('uElapsed', elapsed);
    state.setUniform('uBreath', ghostBreath(elapsed + GHOST_BREATH_LEAD));
    state.setUniform('uIntensity', ghostEntrance(elapsed));
    state.setUniform('uRadius', ghostCornerRadius(state.canvas.width, state.canvas.height, dpr));
    // The theme can change mid-run, and the field has to stay readable in both.
    state.setUniform('uGain', ghostGain(document.documentElement.dataset.theme === 'dark'));
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
