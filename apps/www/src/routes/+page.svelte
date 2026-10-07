<script lang="ts">
  import Header from '$lib/Header.svelte';
  import Hero from '$lib/Hero.svelte';
  import Film from '$lib/Film.svelte';
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
  <meta name="description" content="A motion editor with AI in the loop and you in the driver’s seat. Real layers, editable keyframes, and an editor you can rewrite." />
  <link rel="canonical" href="https://trypowermove.com/" />
  <meta property="og:type" content="website" />
  <meta property="og:site_name" content="Powermove" />
  <meta property="og:url" content="https://trypowermove.com/" />
  <meta property="og:title" content="Powermove" />
  <meta property="og:description" content="A motion editor with AI in the loop and you in the driver’s seat. Real layers, editable keyframes, and an editor you can rewrite." />
  <meta property="og:image" content="https://trypowermove.com/og.png" />
  <meta property="og:image:width" content="1200" />
  <meta property="og:image:height" content="630" />
  <meta property="og:image:alt" content="The Powermove editor with a layer stack, inspector, agent panel, and keyframe timeline." />
  <meta name="twitter:card" content="summary_large_image" />
  <meta name="twitter:title" content="Powermove" />
  <meta name="twitter:description" content="A motion editor with AI in the loop and you in the driver’s seat. Real layers, editable keyframes, and an editor you can rewrite." />
  <meta name="twitter:image" content="https://trypowermove.com/og.png" />
</svelte:head>

<Header onDark />

<main id="top">
  <div class="top">
    <div class="top-glow" aria-hidden="true"></div>
    <section class="hero" aria-label="Introduction">
      <Hero />
    </section>
    <div class="cards">
      <Film />
    </div>
  </div>

  <div class="cards" id="features">
    <article class="card" id="editor" aria-labelledby="editor-title">
      <header class="card-head">
        <div>
          <h2 id="editor-title">A real motion editor</h2>
          <p>Layers, keyframes, curves, and effects on a timeline, with the agent in a panel beside them.</p>
        </div>
      </header>
      <div class="card-body">
        <img src="/editor-hero.png" alt="The Powermove editor in its dark theme over a desert night wallpaper: media, the agent conversation, a canvas with blur controls, the timeline, properties, and effects." width="1920" height="1280" loading="lazy" />
      </div>
    </article>

    <Features />

    <article class="card" aria-labelledby="kernel-title">
      <header class="card-head">
        <div>
          <h2 id="kernel-title">A deliberately tiny kernel</h2>
          <p>Everything above it is an extension, including the parts we wrote. The agent builds new ones through the same typed, undoable tool layer.</p>
        </div>
      </header>
      <div class="card-body kernel-body">
        <div class="stack" bind:this={stack} data-in={stackIn ? '' : undefined} aria-hidden="true">
          <div class="slab yours" style="--i:4"><span class="slab-name">Your mods</span><span class="slab-note">Yours to add. Same API as everything below</span></div>
          <div class="slab" style="--i:3"><span class="slab-name">Panels</span><span class="slab-note">Timeline, inspector, agent, media</span></div>
          <div class="slab" style="--i:2"><span class="slab-name">Effects and transitions</span><span class="slab-note">Blur, grain, whip pan, dissolve</span></div>
          <div class="slab" style="--i:1"><span class="slab-name">Theme and keymap</span><span class="slab-note">Dusk, Vim, yours</span></div>
          <div class="slab kernel" style="--i:0"><span class="slab-name">Kernel</span><span class="slab-note">Project store, typed edits, undo, compositor</span></div>
        </div>
      </div>
    </article>
  </div>
</main>

<Footer />
