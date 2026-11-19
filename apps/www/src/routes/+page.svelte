<script lang="ts">
  import Header from '$lib/Header.svelte';
  import Hero from '$lib/Hero.svelte';
  import Footer from '$lib/Footer.svelte';
  import Features from '$lib/Features.svelte';
  import Faq from '$lib/Faq.svelte';
  import Seo from '$lib/Seo.svelte';
  import { page } from '$app/state';
  import { resolveDownload } from '$lib/download.svelte';
  import { HOME_FAQ } from '$lib/faq';
  import { HOME_DESCRIPTION, HOME_TITLE, faqPage, organization, softwareApplication, website } from '$lib/site';

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

<Seo
  title={HOME_TITLE}
  description={HOME_DESCRIPTION}
  path="/"
  graph={[organization(), website(), softwareApplication(page.data.download), faqPage(HOME_FAQ, '/')]}
/>

<Header onDark />

<main id="top">
  <div class="top">
    <div class="top-glow" aria-hidden="true"></div>
    <section class="hero" aria-labelledby="headline">
      <h1 id="headline" class="sr-only">Powermove: the video editor you customize with AI</h1>
      <Hero />
    </section>
    <figure class="stage-window">
      <img src="/editor-window.png" alt="The Powermove editor with media, the canvas, a Tracks timeline with video and audio clips, and properties." width="1914" height="1080" fetchpriority="high" />
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

  <Faq items={HOME_FAQ} lead="What Powermove is, and what malleable means." />
</main>

<Footer />
