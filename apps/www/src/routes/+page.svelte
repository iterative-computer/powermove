<script lang="ts">
  import { ArrowDown, ArrowUpRight } from '@lucide/svelte';
  import HeroAnimation from '$lib/HeroAnimation.svelte';
</script>

<svelte:head>
  <title>Powermove — Make your move.</title>
  <meta name="description" content="A motion editor with AI in the loop and you in the driver’s seat. Real layers, editable keyframes, and room for your next big idea. In development." />
</svelte:head>

<main id="top">
  <header class="navigation">
    <a class="logo" href="#top" aria-label="Powermove home"><img src="/powermove-logo.svg" width="28" height="24" alt="" /><span>Powermove</span></a>
    <nav aria-label="Main navigation"><a href="#editor">Editor</a></nav>
    <a class="button" href="#preview">Explore Powermove <ArrowDown size={13} /></a>
  </header>
  <section class="hero" aria-labelledby="headline">
    <h1 id="headline" class="sr-only">Shape your video editor</h1>
    <HeroAnimation />
    <div class="hero-intro">
      <p>Powermove is the versatile video software that lets you shape its feature set.</p>
      <a class="button primary" href="mailto:hello@motioner.app?subject=Powermove%20waitlist">Join the waitlist <ArrowUpRight size={14} /></a>
    </div>
  </section>
  <section class="editor-section" id="editor" aria-label="Powermove editor preview">
    <figure class="editor-window" id="preview">
      <img src="/editor-0.png" alt="Powermove editor with a cinematic title, layer stack, property inspector, AI agent, and keyframe timeline." width="1600" height="1000" loading="lazy" />
    </figure>
  </section>
</main>
