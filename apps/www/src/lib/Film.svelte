<script lang="ts">
  let video: HTMLVideoElement | undefined = $state();
  let playing = $state(true);
  let muted = $state(true);

  $effect(() => {
    if (!video) return;
    if (matchMedia('(prefers-reduced-motion: reduce)').matches) { video.pause(); playing = false; }
    // Off screen, the film rests, so the page isn't decoding video nobody sees.
    const observer = new IntersectionObserver(([entry]) => {
      if (!entry.isIntersecting) video!.pause();
      else if (playing) void video!.play();
    });
    observer.observe(video);
    return () => observer.disconnect();
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
</script>

<article class="card film" id="film" aria-labelledby="film-title">
  <header class="card-head">
    <div>
      <h2 id="film-title">Powermove in thirty seconds</h2>
      <p>The launch film. Sound is up to you.</p>
    </div>
    <div class="card-controls">
      <button class="round" type="button" onclick={toggleSound} aria-pressed={!muted} aria-label={muted ? 'Play with sound' : 'Mute'}>
        {#if muted}
          <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true"><path fill="currentColor" d="M7.3 2.6 4.4 5H2.5A.5.5 0 0 0 2 5.5v5a.5.5 0 0 0 .5.5h1.9l2.9 2.4a.5.5 0 0 0 .8-.4V3a.5.5 0 0 0-.8-.4Z" /><path d="m10.5 6 3 3m0-3-3 3" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" /></svg>
        {:else}
          <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true"><path fill="currentColor" d="M7.3 2.6 4.4 5H2.5A.5.5 0 0 0 2 5.5v5a.5.5 0 0 0 .5.5h1.9l2.9 2.4a.5.5 0 0 0 .8-.4V3a.5.5 0 0 0-.8-.4Z" /><path d="M10.3 5.6a3.4 3.4 0 0 1 0 4.8M12.2 3.8a6 6 0 0 1 0 8.4" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" /></svg>
        {/if}
      </button>
      <button class="round" type="button" onclick={togglePlay} aria-label={playing ? 'Pause the film' : 'Play the film'}>
        {#if playing}
          <svg viewBox="0 0 16 16" width="12" height="12" aria-hidden="true"><path fill="currentColor" d="M4 2.5h2.6v11H4zM9.4 2.5H12v11H9.4z" /></svg>
        {:else}
          <svg viewBox="0 0 16 16" width="12" height="12" aria-hidden="true"><path fill="currentColor" d="M4.5 2.8v10.4a.7.7 0 0 0 1.06.6l8.2-5.2a.7.7 0 0 0 0-1.2l-8.2-5.2a.7.7 0 0 0-1.06.6Z" /></svg>
        {/if}
      </button>
    </div>
  </header>
  <div class="card-body">
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
  </div>
</article>
