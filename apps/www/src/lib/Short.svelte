<script lang="ts">
  import type { Clone } from './clones';

  let { clone }: { clone: Clone } = $props();

  let video = $state<HTMLVideoElement | undefined>();

  // Nothing downloads until the short is on screen; then it plays muted and
  // loops, and pauses again when scrolled away. Reduced motion keeps the
  // poster and leaves playback to the controls.
  $effect(() => {
    const el = video;
    if (!el || matchMedia('(prefers-reduced-motion: reduce)').matches) return;
    const observer = new IntersectionObserver(([entry]) => {
      if (entry.isIntersecting) void el.play().catch(() => {});
      else el.pause();
    }, { threshold: 0.4 });
    observer.observe(el);
    return () => observer.disconnect();
  });
</script>

<figure class="short card">
  {#if clone.video}
    <video
      bind:this={video}
      src={clone.video.src}
      poster={clone.video.poster}
      width="720"
      height="1280"
      preload="none"
      muted
      loop
      playsinline
      controls
      aria-label="Short: {clone.video.description}"
    ></video>
    <figcaption class="sr-only">{clone.video.description}</figcaption>
  {:else}
    <div class="short-soon">
      <span class="short-glow" aria-hidden="true"></span>
      <p><span class="strong">Short in production.</span> The video of this prompt running in Powermove lands here soon.</p>
    </div>
  {/if}
</figure>

<style>
  .short{position:relative;aspect-ratio:9/16;width:100%;max-width:400px;justify-self:center;background:#0C0A09}
  .short video{width:100%;height:100%;object-fit:cover}
  .short-soon{position:absolute;inset:0;display:grid;align-content:end;padding:28px;color:rgb(255 255 255 / .62);font-size:15px;line-height:1.5;text-wrap:pretty}
  .short-soon .strong{color:#fff;font-weight:500}
  .short-glow{position:absolute;inset:0;z-index:0;background:radial-gradient(60% 40% at 50% 38%,color-mix(in oklab,var(--ember) 55%,transparent),transparent 70%)}
  .short-soon p{position:relative}
</style>
