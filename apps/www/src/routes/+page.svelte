<script lang="ts">
  import Header from '$lib/Header.svelte';
  import Hero from '$lib/Hero.svelte';
  import Footer from '$lib/Footer.svelte';
  import Features from '$lib/Features.svelte';
  import Faq from '$lib/Faq.svelte';
  import Seo from '$lib/Seo.svelte';
  import { page } from '$app/state';
  import Pricing from '$lib/Pricing.svelte';
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
      { threshold: 0.35 },
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
    <Hero />
    <figure class="stage-window">
      <picture>
        <source srcset="/editor-window.webp" type="image/webp" />
        <img
          src="/editor-window.png"
          alt="The Powermove editor with a project called Summer in Lisbon: a graded sunset over Lisbon with the title in the viewer, a Layers timeline with captions, title keyframes, lower thirds and shots, a Grade layer’s Color Wheels, Vignette and Film Grain, and an Aerolite agent turn that warmed the sunset and added a glow to the title."
          width="1598"
          height="1030"
          fetchpriority="high"
        />
      </picture>
    </figure>
  </div>

  <section class="rows" id="features" aria-labelledby="features-title">
    <h2 id="features-title" class="rows-title"><span class="strong">Welcome to malleable software.</span></h2>

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

  <Pricing />
  <Faq items={HOME_FAQ} title="Questions and answers." />
</main>

<Footer />
