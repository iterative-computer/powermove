<script lang="ts">
  import { onMount, untrack } from 'svelte';

  type Player = { duration: number; seek(time: number): void; destroy(): void };
  type Clip = { name: string; from: number; to: number };

  const FPS = 30;
  const DURATION = 15;
  // Where /hero/scene.json really cuts, so the clips line up with what plays.
  const tracks: { name: string; clips: Clip[] }[] = [
    { name: 'Shape', clips: [{ name: 'Shape', from: 0, to: 15 }] },
    { name: 'your', clips: [{ name: 'your', from: 0, to: 15 }] },
    {
      name: 'Noun',
      clips: [
        { name: 'video editor', from: 0, to: 3 },
        { name: 'gradient editor', from: 3, to: 6 },
        { name: '3D system', from: 6, to: 9 },
        { name: 'effect', from: 9, to: 12 },
        { name: 'video editor', from: 12, to: 15 },
      ],
    },
  ];
  const ticks = Array.from({ length: DURATION + 1 }, (_, i) => i);

  let root: SVGSVGElement | undefined = $state();
  let lanes: HTMLElement | undefined = $state();
  let ready = $state(false);
  let time = $state(0);
  let playing = $state(true);
  let scrubbing = $state(false);

  const frames = $derived(Math.round(time * FPS));
  const timecode = $derived(
    [Math.floor(frames / FPS / 60), Math.floor(frames / FPS) % 60, frames % FPS]
      .map(n => String(n).padStart(2, '0')).join(':'),
  );

  let seek: (t: number) => void = () => {};
  let sync = () => {};

  onMount(() => {
    let cancelled = false, visible = true, frame = 0, last = 0;
    let player: Player | undefined;
    const abort = new AbortController();
    const motion = matchMedia('(prefers-reduced-motion: reduce)');
    const svg = root!;
    // Reduced motion lands on the final hold, which reads as the full headline.
    if (motion.matches) { playing = false; time = DURATION - 1; }

    function fail(error: unknown) {
      if (cancelled) return;
      cancelAnimationFrame(frame);
      player?.destroy(); player = undefined;
      ready = false;
      console.error('[Powermove hero]', error);
    }
    seek = t => {
      time = Math.min(Math.max(t, 0), DURATION - 1 / FPS);
      try { player?.seek(time); } catch (error) { fail(error); }
    };
    function tick(now: number) {
      if (!player) return;
      if (last) time = (time + (now - last) / 1000) % DURATION;
      last = now;
      try { player.seek(time); frame = requestAnimationFrame(tick); }
      catch (error) { fail(error); }
    }
    sync = () => {
      cancelAnimationFrame(frame); last = 0;
      if (!player || cancelled) return;
      if (playing && !scrubbing && visible && !document.hidden) frame = requestAnimationFrame(tick);
      else seek(time);
    };
    const observer = new IntersectionObserver(([entry]) => { visible = entry.isIntersecting; sync(); });
    observer.observe(svg);
    document.fonts.addEventListener('loadingdone', sync);
    document.addEventListener('visibilitychange', sync);
    void (async () => {
      try {
        const [{ createSvgPlayer }, response] = await Promise.all([
          import('$lib/player'), fetch('/hero/scene.json', { signal: abort.signal }),
        ]);
        if (!response.ok) throw new Error('Scene unavailable');
        const scene = await response.json();
        // A slow font must not hold playback. Each seek remeasures glyphs,
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
        player.seek(time); ready = true; sync();
      } catch (error) { fail(error); }
    })();
    return () => {
      cancelled = true; abort.abort(); cancelAnimationFrame(frame); observer.disconnect();
      document.removeEventListener('visibilitychange', sync);
      document.fonts.removeEventListener('loadingdone', sync);
      player?.destroy();
    };
  });

  // Declared after onMount so it first runs once the player wiring exists.
  $effect(() => { void playing; void scrubbing; untrack(sync); });

  function timeAt(x: number) {
    const box = lanes!.getBoundingClientRect();
    return ((x - box.left) / box.width) * DURATION;
  }
  function scrubStart(event: PointerEvent) {
    if (event.button !== 0) return;
    lanes!.setPointerCapture(event.pointerId);
    scrubbing = true;
    seek(timeAt(event.clientX));
  }
  function scrubMove(event: PointerEvent) {
    if (scrubbing) seek(timeAt(event.clientX));
  }
  function scrubEnd() { scrubbing = false; }
  function keys(event: KeyboardEvent) {
    const step = event.shiftKey ? 1 : 1 / FPS;
    const to = { ArrowLeft: time - step, ArrowRight: time + step, Home: 0, End: DURATION }[event.key];
    if (to !== undefined) { playing = false; seek(to); }
    else if (event.key === ' ' || event.key === 'k') playing = !playing;
    else return;
    event.preventDefault();
  }
</script>

<div class="hero-art">
  <div class="artboard-label" aria-hidden="true">
    <span>Powermove header</span>
    <span class="tabular">1920 × 1080</span>
  </div>
  <div class="artboard" role="img" aria-label="Shape your video editor, gradient editor, 3D system, effect. Animated type made in Powermove.">
    {#if !ready}
      <!-- The player's opening hold, laid out as separate layers so the handoff
           keeps its word spacing, anchors, and baselines. -->
      <svg class="hero-vector" viewBox="0 0 1920 1080" preserveAspectRatio="xMidYMid slice" aria-hidden="true">
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

  <div class="timeline" style:--p={time / DURATION} data-scrubbing={scrubbing ? '' : undefined}>
    <div class="transport">
      <button class="play" type="button" aria-label={playing ? 'Pause' : 'Play'} aria-pressed={playing} onclick={() => (playing = !playing)}>
        {#if playing}
          <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true"><path fill="currentColor" d="M4.5 3h2v10h-2zM9.5 3h2v10h-2z" /></svg>
        {:else}
          <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true"><path fill="currentColor" d="M5 3.2v9.6a.6.6 0 0 0 .9.5l7.6-4.8a.6.6 0 0 0 0-1L5.9 2.7a.6.6 0 0 0-.9.5Z" /></svg>
        {/if}
      </button>
      <span class="timecode tabular" aria-hidden="true">{timecode}</span>
    </div>
    <ul class="track-names" aria-hidden="true">
      {#each tracks as track (track.name)}<li>{track.name}</li>{/each}
    </ul>
    <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
    <div
      class="lanes"
      bind:this={lanes}
      role="slider"
      tabindex="0"
      aria-label="Playhead"
      aria-valuemin={0}
      aria-valuemax={DURATION}
      aria-valuenow={Number(time.toFixed(2))}
      aria-valuetext={timecode}
      onpointerdown={scrubStart}
      onpointermove={scrubMove}
      onpointerup={scrubEnd}
      onpointercancel={scrubEnd}
      onkeydown={keys}
    >
      <div class="ruler" aria-hidden="true">
        {#each ticks as t (t)}
          <span class="tick" class:major={t % 3 === 0} style:--t={t / DURATION}>{#if t % 3 === 0 && t < DURATION}<span class="tabular">00:{String(t).padStart(2, '0')}</span>{/if}</span>
        {/each}
      </div>
      {#each tracks as track (track.name)}
        <div class="lane" aria-hidden="true">
          {#each track.clips as clip (clip.from)}
            <span class="clip" style:--from={clip.from / DURATION} style:--len={(clip.to - clip.from) / DURATION} data-active={time >= clip.from && time < clip.to ? '' : undefined}>{clip.name}</span>
          {/each}
        </div>
      {/each}
      <div class="playhead" aria-hidden="true"></div>
    </div>
  </div>
</div>
