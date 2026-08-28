<script lang="ts">
  import { onDestroy } from 'svelte';
  import { useFrame } from '@motion-core/motion-gpu/svelte';
  import {
    RIPPLE_SETTLE_SECONDS,
    rippleFollow,
    rippleIntensity,
  } from './ripple-material';

  let {
    origin,
    sceneBitmap,
    onFirstFrame,
    onSettled,
  }: {
    origin: { x: number; y: number };
    sceneBitmap: ImageBitmap | null;
    onFirstFrame?: () => void;
    onSettled?: () => void;
  } = $props();

  let startedAt: number | null = null;
  let firstFrame = true;
  let sceneBound = false;
  let settled = false;
  let trackingInitialized = false;
  const trackedOrigin = { x: 0, y: 0 };

  const frame = useFrame((state) => {
    if (startedAt == null) startedAt = state.time;
    if (!trackingInitialized) {
      trackedOrigin.x = origin.x;
      trackedOrigin.y = origin.y;
      trackingInitialized = true;
    }
    const elapsed = Math.min(Math.max(0, state.time - startedAt), RIPPLE_SETTLE_SECONDS);
    trackedOrigin.x = rippleFollow(trackedOrigin.x, origin.x, state.delta);
    trackedOrigin.y = rippleFollow(trackedOrigin.y, origin.y, state.delta);

    const rect = state.canvas.getBoundingClientRect();
    const scaleX = state.canvas.width / Math.max(1, rect.width);
    const scaleY = state.canvas.height / Math.max(1, rect.height);
    state.setUniform('uElapsed', elapsed);
    state.setUniform('uIntensity', rippleIntensity(elapsed));
    state.setUniform('uHasScene', sceneBitmap ? 1 : 0);
    state.setUniform('uOrigin', [
      (trackedOrigin.x - rect.left) * scaleX,
      (rect.bottom - trackedOrigin.y) * scaleY,
    ]);

    if (!sceneBound) {
      state.setTexture('uScene', sceneBitmap);
      sceneBound = true;
    }
    if (firstFrame) {
      firstFrame = false;
      onFirstFrame?.();
    }
    if (elapsed >= RIPPLE_SETTLE_SECONDS && !settled) {
      settled = true;
      onSettled?.();
      frame.stop();
    }
  });

  onDestroy(() => sceneBitmap?.close?.());
</script>
