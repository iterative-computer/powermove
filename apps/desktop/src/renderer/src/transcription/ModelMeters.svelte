<script lang="ts">
  /* Speed and accuracy as two short tracks, the way Handy's picker compares
     models. Relative scores (0..1), so only the length carries meaning. */
  let { speed, accuracy }: { speed: number; accuracy: number } = $props();
  const clamp = (value: number) => Math.max(0, Math.min(1, value));
</script>

<span class="meters">
  <span class="meter" role="meter" aria-label="Speed" aria-valuemin="0" aria-valuemax="100" aria-valuenow={Math.round(clamp(speed) * 100)}>
    <span class="label">Speed</span>
    <span class="track"><span class="fill" style:width={`${clamp(speed) * 100}%`}></span></span>
  </span>
  <span class="meter" role="meter" aria-label="Accuracy" aria-valuemin="0" aria-valuemax="100" aria-valuenow={Math.round(clamp(accuracy) * 100)}>
    <span class="label">Accuracy</span>
    <span class="track"><span class="fill" style:width={`${clamp(accuracy) * 100}%`}></span></span>
  </span>
</span>

<style>
  .meters { display: grid; grid-template-columns: auto 44px; gap: 5px 8px; align-items: center; flex: none; }
  .meter { display: contents; }
  .label { color: var(--tx-4); font-size: var(--fs-xs); line-height: 12px; text-align: right; }
  .track { position: relative; height: 3px; border-radius: 2px; background: var(--ink-2); overflow: hidden; }
  .fill { position: absolute; inset: 0 auto 0 0; border-radius: inherit; background: var(--tx-3); }
</style>
