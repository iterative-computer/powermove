<script lang="ts">
  import Header from '$lib/Header.svelte';
  import Hero from '$lib/Hero.svelte';
  import Footer from '$lib/Footer.svelte';
  import Features from '$lib/Features.svelte';
  import { resolveDownload } from '$lib/download.svelte';


  $effect(() => { resolveDownload(); });

  let stack = $state<HTMLElement | undefined>();
  let stackIn = $state(false);

  $effect(() => {
    if (!stack) return;
    const observer = new IntersectionObserver(
      ([entry]) => { if (entry.isIntersecting) { stackIn = true; observer.disconnect(); } },
      { threshold: 0.5 },
    );
    observer.observe(stack);
    return () => observer.disconnect();
  });
</script>

<svelte:head>
  <title>Powermove</title>
  <meta name="description" content="Powermove is a video editing app that lets you shape the featureset. Simply connect an agent and ask." />
  <link rel="canonical" href="https://trypowermove.com/" />
  <meta property="og:type" content="website" />
  <meta property="og:site_name" content="Powermove" />
  <meta property="og:url" content="https://trypowermove.com/" />
  <meta property="og:title" content="Powermove" />
  <meta property="og:description" content="Powermove is a video editing app that lets you shape the featureset. Simply connect an agent and ask." />
  <meta property="og:image" content="https://trypowermove.com/og.png?v=2" />
  <meta property="og:image:width" content="2400" />
  <meta property="og:image:height" content="1260" />
  <meta property="og:image:alt" content="Shape your video editor. The Powermove editor on a warm gradient, with its media, agent conversation, canvas, and keyframe timeline." />
  <meta name="twitter:card" content="summary_large_image" />
  <meta name="twitter:title" content="Powermove" />
  <meta name="twitter:description" content="Powermove is a video editing app that lets you shape the featureset. Simply connect an agent and ask." />
  <meta name="twitter:image" content="https://trypowermove.com/og.png?v=2" />
</svelte:head>

<Header onDark />

<main id="top">
  <div class="top">
    <div class="top-glow" aria-hidden="true"></div>
    <section class="hero" aria-labelledby="headline">
      <h1 id="headline" class="sr-only">Shape your video editor</h1>
      <Hero />
    </section>
    <figure class="stage-window">
      <img src="/editor-window.png" alt="The Powermove editor: media, the agent conversation, the Change literally anything composition on the canvas, its timeline, properties, and effects." width="1782" height="1168" fetchpriority="high" />
    </figure>
  </div>

  <section class="rows" id="features" aria-labelledby="features-title">
    <h2 id="features-title" class="rows-title"><span class="strong">Welcome to malleable software.</span> A real motion editor, with an agent that adds what it’s missing.</h2>

    <article class="row" id="editor" aria-labelledby="editor-title">
      <div class="row-copy">
        <h3 id="editor-title">A real motion editor</h3>
        <p><span class="strong">Layers, keyframes, curves, and effects.</span> All on a timeline, with the agent in a panel beside them.</p>
      </div>
      <div class="row-media"><img src="/editor-hero.png" alt="The Powermove editor in its dark theme over a desert night wallpaper: media, the agent conversation, a canvas with blur controls, the timeline, properties, and effects." width="1920" height="1280" loading="lazy" /></div>
    </article>

    <Features />

    <article class="row" aria-labelledby="kernel-title">
      <div class="row-copy">
        <h3 id="kernel-title">A deliberately tiny kernel</h3>
        <p><span class="strong">Everything above it is an extension,</span> including the parts we wrote. The agent builds new ones through the same typed, undoable tool layer.</p>
      </div>
      <div class="row-media kernel-body">
        <div class="stack" bind:this={stack} data-in={stackIn ? '' : undefined} aria-hidden="true">
          <div class="slab yours" style="--i:4"><span class="slab-name">Your mods</span><span class="slab-note">Yours to add. Same API as everything below</span></div>
          <div class="slab" style="--i:3"><span class="slab-name">Panels</span><span class="slab-note">Timeline, inspector, agent, media</span></div>
          <div class="slab" style="--i:2"><span class="slab-name">Effects and transitions</span><span class="slab-note">Blur, grain, whip pan, dissolve</span></div>
          <div class="slab" style="--i:1"><span class="slab-name">Theme and keymap</span><span class="slab-note">Dusk, Vim, yours</span></div>
          <div class="slab kernel" style="--i:0"><span class="slab-name">Kernel</span><span class="slab-note">Project store, typed edits, undo, compositor</span></div>
        </div>
      </div>
    </article>
  </section>
</main>

<Footer />
