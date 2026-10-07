<script lang="ts">
  const FPS = 30;

  let video: HTMLVideoElement | undefined = $state();
  let bar: HTMLElement | undefined = $state();
  let playing = $state(true);
  let muted = $state(true);
  let time = $state(0);
  let duration = $state(30);
  let scrubbing = $state(false);

  const frames = $derived(Math.round(time * FPS));
  const timecode = $derived(
    [Math.floor(frames / FPS / 60), Math.floor(frames / FPS) % 60, frames % FPS]
      .map(n => String(n).padStart(2, '0')).join(':'),
  );

  $effect(() => {
    if (!video) return;
    const v = video;
    if (matchMedia('(prefers-reduced-motion: reduce)').matches) { v.pause(); playing = false; }
    // The transport reads the clock every frame while the film runs; timeupdate is too coarse for frames.
    let frame = 0;
    const read = () => { time = v.currentTime; if (!v.paused) frame = requestAnimationFrame(read); };
    const onPlay = () => { cancelAnimationFrame(frame); frame = requestAnimationFrame(read); };
    const onMeta = () => { duration = v.duration || duration; };
    v.addEventListener('play', onPlay);
    v.addEventListener('loadedmetadata', onMeta);
    if (v.readyState >= 1) onMeta();
    if (!v.paused) onPlay();
    // Off screen, the film rests, so the page isn't decoding video nobody sees.
    const observer = new IntersectionObserver(([entry]) => {
      if (!entry.isIntersecting) v.pause();
      else if (playing) void v.play();
    });
    observer.observe(v);
    return () => {
      cancelAnimationFrame(frame); observer.disconnect();
      v.removeEventListener('play', onPlay); v.removeEventListener('loadedmetadata', onMeta);
    };
  });

  function togglePlay() {
    playing = !playing;
    if (playing) void video!.play(); else video!.pause();
  }
  function toggleSound() {
    muted = !muted;
    video!.muted = muted;
    // Sound starts the film from the top, so the music lands with the picture.
    if (!muted) { video!.currentTime = 0; playing = true; void video!.play(); }
  }
  function seekTo(x: number) {
    const box = bar!.getBoundingClientRect();
    video!.currentTime = time = Math.min(Math.max((x - box.left) / box.width, 0), 1) * duration;
  }
  function keys(event: KeyboardEvent) {
    const step = event.shiftKey ? 1 : 1 / FPS;
    const to = { ArrowLeft: time - step, ArrowRight: time + step, Home: 0, End: duration - 1 / FPS }[event.key];
    if (to === undefined) return;
    event.preventDefault();
    video!.currentTime = time = Math.min(Math.max(to, 0), duration);
  }
</script>

<figure class="window" id="film">
  <div class="window-bar" aria-hidden="true">
    <span class="lights"><i></i><i></i><i></i></span>
    <span class="window-title">Powermove launch film</span>
  </div>
  <video
    bind:this={video}
    src="/launch.mp4"
    poster="/launch-poster.jpg"
    autoplay
    muted
    loop
    playsinline
    preload="metadata"
    width="1920"
    height="1080"
    aria-label="Powermove launch film"
  ></video>
  <figcaption class="transport" style:--p={time / duration} data-scrubbing={scrubbing ? '' : undefined}>
    <button class="t-button" type="button" onclick={togglePlay} aria-label={playing ? 'Pause the film' : 'Play the film'}>
      {#if playing}
        <svg viewBox="0 0 16 16" width="12" height="12" aria-hidden="true"><path fill="currentColor" d="M4.5 3h2v10h-2zM9.5 3h2v10h-2z" /></svg>
      {:else}
        <svg viewBox="0 0 16 16" width="12" height="12" aria-hidden="true"><path fill="currentColor" d="M5 3.2v9.6a.6.6 0 0 0 .9.5l7.6-4.8a.6.6 0 0 0 0-1L5.9 2.7a.6.6 0 0 0-.9.5Z" /></svg>
      {/if}
    </button>
    <span class="t-time tabular" aria-hidden="true">{timecode}</span>
    <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
    <div
      class="t-bar"
      bind:this={bar}
      role="slider"
      tabindex="0"
      aria-label="Film position"
      aria-valuemin={0}
      aria-valuemax={Math.round(duration)}
      aria-valuenow={Math.round(time)}
      aria-valuetext={timecode}
      onpointerdown={event => { bar!.setPointerCapture(event.pointerId); scrubbing = true; seekTo(event.clientX); }}
      onpointermove={event => scrubbing && seekTo(event.clientX)}
      onpointerup={() => (scrubbing = false)}
      onpointercancel={() => (scrubbing = false)}
      onkeydown={keys}
    >
      <span class="t-track"></span>
      <span class="t-head"></span>
    </div>
    <button class="t-button" type="button" onclick={toggleSound} aria-pressed={!muted} aria-label={muted ? 'Play with sound' : 'Mute'}>
      {#if muted}
        <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true"><path fill="currentColor" d="M7.3 2.6 4.4 5H2.5A.5.5 0 0 0 2 5.5v5a.5.5 0 0 0 .5.5h1.9l2.9 2.4a.5.5 0 0 0 .8-.4V3a.5.5 0 0 0-.8-.4Z" /><path d="m10.5 6 3 3m0-3-3 3" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" /></svg>
      {:else}
        <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true"><path fill="currentColor" d="M7.3 2.6 4.4 5H2.5A.5.5 0 0 0 2 5.5v5a.5.5 0 0 0 .5.5h1.9l2.9 2.4a.5.5 0 0 0 .8-.4V3a.5.5 0 0 0-.8-.4Z" /><path d="M10.3 5.6a3.4 3.4 0 0 1 0 4.8M12.2 3.8a6 6 0 0 1 0 8.4" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" /></svg>
      {/if}
    </button>
  </figcaption>
</figure>
