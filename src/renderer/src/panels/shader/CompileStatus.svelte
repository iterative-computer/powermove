<script lang="ts">
  import type { ShaderLayer } from '../../core/types/project';
  import { frameBus } from '../../runtime/frame-bus';
  import { doc } from '../../state/document.svelte';

  let {
    PM,
    layer,
    refreshToken = 0
  }: {
    PM: Record<string, any>;
    layer: ShaderLayer;
    refreshToken?: number;
  } = $props();

  let message = $state('ready');
  let failed = $state(false);

  function refresh(): void {
    const meta = PM.UIState.getShaderMeta(layer);
    const error = PM.GL.compileError(meta.shaderKey);
    failed = !!error;
    message = error ? (String(error).split('\n')[0]?.slice(0, 90) ?? '') : `✓ compiled · ${meta.udefs?.length ?? 0} uniforms`;
  }

  /* Source edits invalidate the compositor first. Read once immediately, then
     again on the next frame after GL has had a chance to compile the new key.
     The low-frequency status bus covers later compositor-only changes. */
  $effect(() => {
    doc.tick.values;
    refreshToken;
    layer.id;
    refresh();
    const frame = window.requestAnimationFrame(refresh);
    return () => window.cancelAnimationFrame(frame);
  });

  $effect(() => frameBus.on('status', refresh));
</script>

<span
  class:bad={failed}
  class:ok={!failed}
  class="compile-status"
  role="status"
  aria-live="polite"
  aria-atomic="true"
  title={message}
>{message}</span>

<style>
  .compile-status {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
