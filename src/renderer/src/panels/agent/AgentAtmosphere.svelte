<script lang="ts">
  import { mountGpuField } from './gpu-field';

  /** Decorative composition planes; the adjacent copy owns the activity label. */
  let { active = false, compact = false }: { active?: boolean; compact?: boolean } = $props();

  let element = $state<HTMLElement>();
  let visible = $state(false);

  // Do not even allocate the optional renderer until this emblem is visible.
  // This owner releases the shared renderer's resources when the run or view
  // ends, including when the document is backgrounded or the mark is scrolled.
  $effect(() => {
    const host = element;
    if (!host) return;
    let onScreen = typeof IntersectionObserver === 'undefined';
    const update = () => { visible = onScreen && !document.hidden; };
    const observer = typeof IntersectionObserver === 'undefined' ? null : new IntersectionObserver(entries => {
      onScreen = entries.some(entry => entry.isIntersecting);
      update();
    });
    observer?.observe(host);
    document.addEventListener('visibilitychange', update);
    update();
    return () => {
      observer?.disconnect();
      document.removeEventListener('visibilitychange', update);
    };
  });

  const atmosphere = (host: HTMLElement) => ({
    destroy: mountGpuField(host, () => import('./AgentAtmosphereCanvas.svelte'), {
      rendererKey: 'atmosphereRenderer',
      fallbackKey: 'atmosphereFallback',
      label: 'Agent atmosphere',
      // At most 52 × 30 pixels: this mark sits beside a live composition.
      maxDevicePixelRatio: 1,
    }),
  });
</script>

<span class="agent-atmosphere" class:compact class:active bind:this={element} aria-hidden="true" data-agent-atmosphere data-active={active}>
  <span class="atmosphere-light">
    {#if active && visible}
      <span class="atmosphere-field" use:atmosphere></span>
    {/if}
  </span>
  <svg viewBox="0 0 72 44" fill="none" focusable="false">
    <path class="plane-back" d="M12 12.5 38 4 60 14 34 22.5Z" />
    <path class="plane-middle" d="M12 21.5 38 13 60 23 34 31.5Z" />
    <path class="plane-front" d="M12 30.5 38 22 60 32 34 40.5Z" />
    <path class="plane-crest" d="m12 21.5 22 10 26-8.5" />
  </svg>
</span>

<style>
  .agent-atmosphere {
    position: relative;
    display: inline-block;
    flex: none;
    width: 72px;
    height: 44px;
    overflow: hidden;
    contain: layout paint;
    isolation: isolate;
    pointer-events: none;
    user-select: none;
    color: var(--tx-3, #94949a);
  }
  .compact { width: 28px; height: 20px; }
  svg { position: absolute; inset: 0; width: 100%; height: 100%; overflow: visible; }
  svg path { stroke: currentColor; stroke-width: 1; stroke-linecap: round; stroke-linejoin: round; }
  .plane-back { opacity: .28; }
  .plane-middle { opacity: .6; }
  .plane-front { opacity: .28; }
  .plane-crest { opacity: 0; transition: opacity var(--dur-2, 160ms); }
  .active .plane-crest { opacity: .8; stroke: #94bac8; }
  .atmosphere-light {
    position: absolute;
    inset: 7px 10px;
    overflow: hidden;
    opacity: .28;
    background: radial-gradient(ellipse at 62% 55%, #6ba9bd33, #797eaa22 42%, transparent 72%);
    mask-image: radial-gradient(ellipse, #000 12%, transparent 72%);
    transition: opacity var(--dur-2, 160ms);
  }
  .active .atmosphere-light { opacity: .9; }
  .atmosphere-field {
    position: absolute;
    inset: 0;
    opacity: .75;
  }
  .atmosphere-field :global(canvas) { display: block; width: 100%; height: 100%; pointer-events: none; }
  .compact .atmosphere-light { inset: 2px; }
  .compact .plane-back, .compact .plane-front { opacity: .4; }
  .compact svg path { stroke-width: 1.6; }
  @media (prefers-reduced-motion: reduce) {
    .plane-crest, .atmosphere-light { transition: none; }
    .atmosphere-field { display: none; }
  }
</style>
