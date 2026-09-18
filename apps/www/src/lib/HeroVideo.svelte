<script lang="ts">
  import { Volume2, VolumeX } from '@lucide/svelte';

  let video: HTMLVideoElement;
  let muted = $state(true);

  function toggle() {
    muted = !muted;
    video.muted = muted;
    if (!muted) {
      // Start the ad from the top so the sound lands with the picture.
      video.currentTime = 0;
      void video.play();
    }
  }
</script>

<div class="hero-video">
  <video
    bind:this={video}
    src="/launch.mp4"
    poster="/launch-poster.jpg"
    autoplay
    muted
    loop
    playsinline
    preload="auto"
    width="1920"
    height="1080"
    aria-label="Powermove launch video"
  ></video>
  <button class="sound" type="button" onclick={toggle} aria-pressed={!muted} aria-label={muted ? 'Unmute' : 'Mute'}>
    {#if muted}<VolumeX size={15} strokeWidth={1.75} />{:else}<Volume2 size={15} strokeWidth={1.75} />{/if}
    <span>{muted ? 'Sound' : 'Mute'}</span>
  </button>
</div>

<style>
  .hero-video{position:relative;aspect-ratio:16/9;border-radius:var(--r-xl);box-shadow:var(--shadow-panel);overflow:hidden;background:#f7f7f8;isolation:isolate}
  .hero-video::after{content:'';position:absolute;inset:0;border-radius:inherit;box-shadow:inset 0 0 0 1px rgb(var(--ink-rgb) / .08);pointer-events:none}
  video{display:block;width:100%;height:100%;object-fit:cover}
  .sound{position:absolute;right:14px;bottom:14px;display:inline-flex;align-items:center;gap:7px;height:30px;padding:0 11px 0 9px;border-radius:999px;font-size:12.5px;font-weight:500;letter-spacing:.005em;color:#fff;background:rgb(0 0 0 / .42);backdrop-filter:blur(12px) saturate(1.4);-webkit-backdrop-filter:blur(12px) saturate(1.4);box-shadow:inset 0 0 0 1px rgb(255 255 255 / .14);opacity:.78;transition:opacity var(--dur-2),background var(--dur-2),transform var(--dur-1) var(--ease)}
  .hero-video:hover .sound,.sound:focus-visible{opacity:1}
  .sound:hover{background:rgb(0 0 0 / .56)}
  .sound:active{transform:scale(.97)}
  .sound:focus-visible{outline:2px solid #fff;outline-offset:2px}
  @media(min-width:768px){.sound{right:18px;bottom:18px}}
  @media(prefers-reduced-motion:reduce){.sound{transition:none}}
</style>
