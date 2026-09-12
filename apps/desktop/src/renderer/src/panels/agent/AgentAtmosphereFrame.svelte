<script lang="ts">
  import { useFrame } from '@motion-core/motion-gpu/svelte';
  import { atmosphereUniforms } from './agent-atmosphere-material';

  let { onFirstFrame }: { onFirstFrame?: () => void } = $props();
  let elapsed = 0;
  let firstFrame = true;
  const frame = useFrame(state => {
    elapsed += Math.max(0, state.delta);
    const uniforms = atmosphereUniforms(elapsed, document.documentElement.dataset.theme === 'dark');
    for (const [name, value] of Object.entries(uniforms)) state.setUniform(name, value);
    if (firstFrame) {
      firstFrame = false;
      onFirstFrame?.();
    }
  });
  // AgentAtmosphere owns document/intersection visibility and unmounts this
  // renderer when hidden. Motion GPU owns the frame subscription and teardown.
  $effect(() => {
    frame.start();
    return () => frame.stop();
  });
</script>
