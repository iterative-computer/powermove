<script lang="ts">
  import { onMount } from 'svelte';

  type Player = { duration: number; seek(time: number): void; destroy(): void };

  let root: SVGSVGElement | undefined = $state();
  let ready = $state(false);

  onMount(() => {
    let cancelled = false, visible = true, frame = 0, last = 0, elapsed = 0;
    let player: Player | undefined;
    const abort = new AbortController();
    const motion = matchMedia('(prefers-reduced-motion: reduce)');
    const svg = root!;
    ready = false;
    function fail(error: unknown) {
      if (cancelled) return;
      cancelAnimationFrame(frame);
      player?.destroy(); player = undefined;
      delete svg.dataset.ready;
      ready = false;
      console.error('[Powermove hero]', error);
    }
    function tick(now: number) {
      if (!player) return;
      if (last) elapsed = (elapsed + (now - last) / 1000) % player.duration;
      last = now;
      try { player.seek(elapsed); frame = requestAnimationFrame(tick); }
      catch (error) { fail(error); }
    }
    function sync() {
      cancelAnimationFrame(frame); last = 0;
      if (!player || cancelled) return;
      if (motion.matches) player.seek(Math.max(0, player.duration - 1));
      else if (visible && !document.hidden) frame = requestAnimationFrame(tick);
    }
    const observer = new IntersectionObserver(([entry]) => { visible = entry.isIntersecting; sync(); });
    observer.observe(svg);
    motion.addEventListener('change', sync);
    document.fonts.addEventListener('loadingdone', sync);
    document.addEventListener('visibilitychange', sync);
    void (async () => {
      try {
        const [{ createSvgPlayer }, response] = await Promise.all([
          import('$lib/player'), fetch('/hero/scene.json', { signal: abort.signal }),
        ]);
        if (!response.ok) throw new Error('Scene unavailable');
        const scene = await response.json();
        // A slow font must not prevent playback. Each seek remeasures glyphs,
        // so a font that arrives later is picked up on the next frame.
        let fontTimeout: ReturnType<typeof setTimeout> | undefined;
        try {
          await Promise.race([
            document.fonts.load('400 128px Geist').catch(() => []),
            new Promise(resolve => { fontTimeout = setTimeout(resolve, 1500); }),
          ]);
        } finally { clearTimeout(fontTimeout); }
        if (cancelled) return;
        player = createSvgPlayer(svg, scene);
        player.seek(0); svg.dataset.ready = 'true'; sync(); ready = true;
      } catch (error) { fail(error); }
    })();
    return () => {
      cancelled = true; abort.abort(); cancelAnimationFrame(frame); observer.disconnect();
      motion.removeEventListener('change', sync); document.removeEventListener('visibilitychange', sync);
      document.fonts.removeEventListener('loadingdone', sync);
      player?.destroy();
    };
  });
</script>

<div class="hero-art" role="group" aria-label="Made in Powermove: animated typography showcasing a gradient editor, 3D system, and effect">
  {#if !ready}
    <!-- Match the player's opening hold exactly: separate text layers preserve
         its word spacing, anchor correction, and baselines during handoff. -->
    <svg class="hero-vector hero-poster" viewBox="0 0 1920 1080" preserveAspectRatio="xMidYMid slice" aria-hidden="true">
      <g fill="#F5F5F4" font-family="Geist" font-size="128" font-weight="600" letter-spacing="-4" style="font-variation-settings:'wght' 398.334">
        <g transform="translate(342.9186172485351 470.1759932556153)">
          <text y="104.96" text-anchor="end" transform="translate(309.7305908203125 1.0422963714599547)">Shape</text>
        </g>
        <g transform="translate(686.2853775024414 470.1759932556153)">
          <text y="104.96" text-anchor="start">your</text>
        </g>
        <g transform="translate(967.8048477172852 470.1759932556153)">
          <text y="104.96" text-anchor="start">video editor</text>
        </g>
      </g>
    </svg>
  {/if}
  <svg bind:this={root} class="hero-vector" viewBox="0 0 1920 1080" preserveAspectRatio="xMidYMid slice" aria-hidden="true" style:visibility={ready ? 'visible' : 'hidden'}></svg>
</div>
